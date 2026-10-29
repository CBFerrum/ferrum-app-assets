//! Maintenance plans: tasks with a periodicity, an assignee and a next due date, and the logs
//! that record each occurrence. A task's next due date counts one period from its start or from
//! its last completion, whichever is later (and stops after its end date). Due tasks get a
//! planned log and their assignee a notification (`open_due_tasks`, run by `maintenance`).

use crate::asset::{self, day, in_service};
use crate::named;
use async_trait::async_trait;
use chrono::{Duration, NaiveDate};
use ferrum_domain::util::{get, id, rows, s, set, sys};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result, with_retry};
use serde_json::{Value, json};
use uuid::Uuid;

/// The date one period after `from`.
fn after(periodicity: &str, from: NaiveDate) -> NaiveDate {
    let months = match periodicity {
        "Daily" => return from + Duration::days(1),
        "Weekly" => return from + Duration::days(7),
        "Quarterly" => 3,
        "Half-yearly" => 6,
        "Yearly" => 12,
        "2 Yearly" => 24,
        "3 Yearly" => 36,
        _ => 1,
    };
    crate::schedule::add_months(from, months)
}

/// A task's next due date, or `None` once it is past its end date.
pub(crate) fn next_due(task: &Doc) -> Option<NaiveDate> {
    let start = day(task, "start_date")?;
    let from = day(task, "last_completion_date").filter(|c| *c > start).unwrap_or(start);
    let due = after(s(task, "periodicity").unwrap_or("Monthly"), from);
    match day(task, "end_date") {
        Some(end) if due > end => None,
        _ => Some(due),
    }
}

fn due_value(task: &Doc) -> Value {
    json!(next_due(task).map(|d| d.to_string()))
}

/// Recomputes a task's last completion (from its submitted logs) and next due date.
async fn sync_task(k: &Kernel, ctx: &Ctx, tx: &mut ferrum_kernel::Tx, plan: Uuid, task: &str) -> Result<()> {
    let last: Option<NaiveDate> = sqlx::query_scalar(
        "select max(completion_date) from e_asset_maintenance_log where tenant_id = $1 and asset_maintenance = $2 and task = $3 \
         and docstatus = 1 and maintenance_status = 'Completed'",
    )
    .bind(ctx.tenant)
    .bind(plan)
    .bind(task)
    .fetch_one(&mut **tx)
    .await?;
    let p = get(k, ctx, tx, "AssetMaintenance", plan).await?;
    let Some(mut row) = rows(&p, "tasks").into_iter().find(|r| s(r, "id") == Some(task)) else { return Ok(()) };
    set(&mut row, "last_completion_date", json!(last.map(|d| d.to_string())));
    let mut v = Doc::new();
    v.insert("last_completion_date".into(), row.get("last_completion_date").cloned().unwrap_or(Value::Null));
    v.insert("next_due_date".into(), due_value(&row));
    let row_id = id(&row, "id").ok_or_else(|| Error::internal("task without id"))?;
    k.set_child_values_tx(&sys(ctx), tx, "AssetMaintenance", "tasks", row_id, v).await
}

pub struct PlanHook;

#[async_trait]
impl DocHook for PlanHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if let Some(a) = id(doc, "asset") {
            let current = get(hc.kernel, hc.ctx, hc.tx, "Asset", a).await?;
            set(doc, "company", current.get("company").cloned().unwrap_or(Value::Null));
        }
        let mut tasks = rows(doc, "tasks");
        for t in tasks.iter_mut() {
            let due = due_value(t);
            set(t, "next_due_date", due);
        }
        set(doc, "tasks", Value::Array(tasks.into_iter().map(Value::Object).collect()));
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let a = id(doc, "asset").ok_or_else(|| Error::field("asset", "is required"))?;
        let current = hc.kernel.get_tx(hc.ctx, hc.tx, "Asset", a, false).await?;
        in_service(&current)
    }
}

pub struct LogHook;

#[async_trait]
impl DocHook for LogHook {
    /// While a draft, a log shows its task as the plan has it.
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if doc.get("docstatus").and_then(Value::as_i64).unwrap_or(0) != 0 {
            return Ok(());
        }
        let Some(plan) = id(doc, "asset_maintenance") else { return Ok(()) };
        let p = get(hc.kernel, hc.ctx, hc.tx, "AssetMaintenance", plan).await?;
        set(doc, "asset", p.get("asset").cloned().unwrap_or(Value::Null));
        set(doc, "company", p.get("company").cloned().unwrap_or(Value::Null));
        let task = s(doc, "task").map(str::to_string);
        if let Some(t) = rows(&p, "tasks").into_iter().find(|r| s(r, "id") == task.as_deref()) {
            for (mine, theirs) in [("task_name", "task_name"), ("maintenance_type", "maintenance_type"), ("periodicity", "periodicity")] {
                set(doc, mine, t.get(theirs).cloned().unwrap_or(Value::Null));
            }
            if s(doc, "assigned_to").is_none() {
                set(doc, "assigned_to", t.get("assign_to").cloned().unwrap_or(Value::Null));
            }
            if s(doc, "due_date").is_none() {
                set(doc, "due_date", t.get("next_due_date").cloned().unwrap_or(Value::Null));
            }
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let plan = id(doc, "asset_maintenance").ok_or_else(|| Error::field("asset_maintenance", "is required"))?;
        let p = hc.kernel.get_tx(hc.ctx, hc.tx, "AssetMaintenance", plan, false).await?;
        if !rows(&p, "tasks").iter().any(|r| s(r, "id") == s(doc, "task")) {
            return Err(Error::field("task", "is not a task of this maintenance plan"));
        }
        let current = get(hc.kernel, hc.ctx, hc.tx, "Asset", id(&p, "asset").unwrap_or_default()).await?;
        in_service(&current)?;
        if day(doc, "completion_date").is_some_and(|c| c > hc.ctx.today()) {
            return Err(Error::field("completion_date", "cannot be in the future"));
        }
        Ok(())
    }

    /// A log in progress puts the asset in maintenance (and a changed one may take it out).
    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        for a in [id(doc, "asset"), old.and_then(|o| id(o, "asset"))].into_iter().flatten().collect::<std::collections::BTreeSet<_>>() {
            asset::refresh(hc.kernel, hc.ctx, hc.tx, a, None).await?;
        }
        Ok(())
    }

    async fn before_submit(&self, _hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if !matches!(s(doc, "maintenance_status"), Some("Completed" | "Cancelled")) {
            return Err(Error::field("maintenance_status", "complete or cancel the maintenance before submitting it"));
        }
        Ok(())
    }

    async fn on_submit(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        self.settle(hc, doc).await
    }

    async fn on_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        self.settle(hc, doc).await
    }

    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let (Some(a), Some(me)) = (id(doc, "asset"), id(doc, "id")) {
            asset::refresh(hc.kernel, hc.ctx, hc.tx, a, Some(me)).await?;
        }
        Ok(())
    }
}

impl LogHook {
    /// The task's dates follow its completed logs; the asset's status follows its open logs.
    async fn settle(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let (Some(plan), Some(task)) = (id(doc, "asset_maintenance"), s(doc, "task")) {
            sync_task(hc.kernel, hc.ctx, hc.tx, plan, task).await?;
        }
        if let Some(a) = id(doc, "asset") {
            asset::refresh(hc.kernel, hc.ctx, hc.tx, a, None).await?;
        }
        Ok(())
    }
}

/// The active user signing in with `email`, if any.
pub(crate) async fn user_by_email(tx: &mut ferrum_kernel::Tx, tenant: Uuid, email: &str) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar("select id from app_user where tenant_id = $1 and email = $2 and is_active")
        .bind(tenant)
        .bind(email)
        .fetch_optional(&mut **tx)
        .await?)
}

/// For every task that came due: a planned log for that due date and a notification to the
/// assignee, once per due date. Returns how many tasks were opened.
pub(crate) async fn open_due_tasks(k: &Kernel, ctx: &Ctx) -> Result<usize> {
    let today = ctx.today();
    let due: Vec<(Uuid, Uuid)> = {
        let mut tx = k.db.tenant_tx(ctx.tenant).await?;
        let v = sqlx::query_as(
            "select m.id, t.id from e_asset_maintenance m join e_asset_maintenance__tasks t on t.tenant_id = m.tenant_id and t.parent_id = m.id \
             join e_asset a on a.tenant_id = m.tenant_id and a.id = m.asset \
             where m.tenant_id = $1 and t.next_due_date <= $2 and (t.last_notified_date is null or t.last_notified_date < t.next_due_date) \
             and a.docstatus = 1 and a.disposal is null order by t.next_due_date limit 200",
        )
        .bind(ctx.tenant)
        .bind(today)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        v
    };
    let mut opened = 0;
    for (plan, task) in due {
        let done = with_retry(|| async {
            let mut tx = k.db.tenant_tx(ctx.tenant).await?;
            let p = k.get_tx(ctx, &mut tx, "AssetMaintenance", plan, true).await?;
            let Some(t) = rows(&p, "tasks").into_iter().find(|r| id(r, "id") == Some(task)) else { return Ok(false) };
            let Some(due) = day(&t, "next_due_date").filter(|d| *d <= today) else { return Ok(false) };
            if day(&t, "last_notified_date").is_some_and(|n| n >= due) {
                return Ok(false);
            }
            let exists: bool = sqlx::query_scalar(
                "select exists(select 1 from e_asset_maintenance_log where tenant_id = $1 and asset_maintenance = $2 and task = $3 and due_date = $4 and docstatus < 2)",
            )
            .bind(ctx.tenant)
            .bind(plan)
            .bind(task.to_string())
            .bind(due)
            .fetch_one(&mut *tx)
            .await?;
            let mut log = None;
            if !exists {
                let input = json!({"asset_maintenance": plan.to_string(), "task": task.to_string(), "due_date": due.to_string(), "maintenance_status": "Planned"});
                let Value::Object(input) = input else { return Ok(false) };
                log = id(&k.insert_tx(ctx, &mut tx, "AssetMaintenanceLog", input).await?, "id");
            }
            if let Some(email) = s(&t, "assign_to")
                && let Some(user) = user_by_email(&mut tx, ctx.tenant, email).await?
            {
                let asset_name = s(&p, "asset_name").map(str::to_string).unwrap_or_else(|| named(&p));
                let title = format!("Maintenance due: {} on {asset_name} ({due})", s(&t, "task_name").unwrap_or("task"));
                let title: String = title.chars().take(200).collect();
                let (entity, record) = match log {
                    Some(l) => ("AssetMaintenanceLog", l),
                    None => ("AssetMaintenance", plan),
                };
                ferrum_kernel::notify::send(&mut tx, ctx.tenant, user, &title, s(&t, "description"), Some(entity), Some(record)).await?;
            }
            let mut v = Doc::new();
            v.insert("last_notified_date".into(), json!(due.to_string()));
            k.set_child_values_tx(ctx, &mut tx, "AssetMaintenance", "tasks", task, v).await?;
            tx.commit().await?;
            Ok(true)
        })
        .await;
        match done {
            Ok(true) => opened += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!(tenant = %ctx.tenant, %plan, "opening a due maintenance task failed: {e}"),
        }
    }
    Ok(opened)
}
