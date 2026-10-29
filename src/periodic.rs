//! The per-minute upkeep, for every active workspace with the package: books the depreciation
//! that fell due (up to the workspace's today) and opens the maintenance tasks that came due.

use crate::{asset, maintenance};
use ferrum_domain::util::id;
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, with_retry};
use serde_json::{Value, json};
use uuid::Uuid;

/// Most rows booked per workspace per round; the rest follow in the next rounds.
const BATCH: usize = 500;

pub(crate) async fn run(k: &Kernel) -> Result<()> {
    let tenants: Vec<Uuid> = {
        let mut tx = k.db.system_tx().await?;
        let t = sqlx::query_scalar("select id from tenant where status = 'active' order by id").fetch_all(&mut *tx).await?;
        tx.commit().await?;
        t
    };
    for tenant in tenants {
        let schema = match k.registry.schema(tenant).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(%tenant, "asset upkeep skipped: {e}");
                continue;
            }
        };
        if !schema.packages.iter().any(|p| p == "assets") {
            continue;
        }
        let ctx = Ctx::system(tenant, schema);
        match depreciate(k, &ctx).await {
            Ok(n) if n > 0 => tracing::info!(%tenant, booked = n, "depreciation booked"),
            Ok(_) => {}
            Err(e) => tracing::warn!(%tenant, "depreciation posting failed: {e}"),
        }
        if let Err(e) = maintenance::open_due_tasks(k, &ctx).await {
            tracing::warn!(%tenant, "opening due maintenance failed: {e}");
        }
    }
    Ok(())
}

/// Books every schedule row due by today, oldest first, one row per transaction: an interrupted
/// round leaves whole rows booked and the next round carries on, and a second run finds nothing
/// left to do. An asset whose row cannot be booked (a closed period, a disabled account) is
/// skipped until the next round; the reason is kept on the row and Asset Managers are told once.
pub(crate) async fn depreciate(k: &Kernel, ctx: &Ctx) -> Result<usize> {
    let today = ctx.today();
    let assets: Vec<Uuid> = {
        let mut tx = k.db.tenant_tx(ctx.tenant).await?;
        let v = sqlx::query_scalar(
            "select a.id from e_asset a join e_asset__schedule s on s.tenant_id = a.tenant_id and s.parent_id = a.id \
             where a.tenant_id = $1 and a.docstatus = 1 and a.disposal is null and not s.booked and s.schedule_date <= $2 \
             group by a.id order by min(s.schedule_date), a.id limit 200",
        )
        .bind(ctx.tenant)
        .bind(today)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        v
    };
    let mut booked = 0;
    for a in assets {
        while booked < BATCH {
            let step = with_retry(|| async {
                let mut tx = k.db.tenant_tx(ctx.tenant).await?;
                let (mut doc, old) = asset::open(k, ctx, &mut tx, a).await?;
                if doc.get("docstatus").and_then(Value::as_i64) != Some(1) || id(&doc, "disposal").is_some() {
                    return Ok(false);
                }
                if !asset::book_next(k, ctx, &mut tx, &mut doc, today).await? {
                    return Ok(false);
                }
                asset::save(k, ctx, &mut tx, doc, &old, None).await?;
                tx.commit().await?;
                Ok(true)
            })
            .await;
            match step {
                Ok(true) => booked += 1,
                Ok(false) => break,
                Err(e) => {
                    record_failure(k, ctx, a, &e).await;
                    break;
                }
            }
        }
    }
    Ok(booked)
}

/// A readable reason, with the fields of a validation failure.
fn reason(e: &Error) -> String {
    match e {
        Error::Validation(fields) => fields.iter().map(|f| format!("{}: {}", f.field, f.message)).collect::<Vec<_>>().join("; "),
        other => other.to_string(),
    }
}

/// Keeps the reason on the asset's next row and tells its Asset Managers when it is new.
async fn record_failure(k: &Kernel, ctx: &Ctx, asset: Uuid, e: &Error) {
    let why: String = reason(e).chars().take(1000).collect();
    tracing::warn!(tenant = %ctx.tenant, %asset, "depreciation could not be booked: {why}");
    let done: Result<()> = async {
        let mut tx = k.db.tenant_tx(ctx.tenant).await?;
        let row: Option<(Uuid, Option<String>, String)> = sqlx::query_as(
            "select s.id, s.posting_error, coalesce(a.number, a.id::text) from e_asset__schedule s join e_asset a on a.tenant_id = s.tenant_id and a.id = s.parent_id \
             where s.tenant_id = $1 and s.parent_id = $2 and not s.booked order by s.schedule_date limit 1",
        )
        .bind(ctx.tenant)
        .bind(asset)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((row, previous, number)) = row else { return Ok(()) };
        if previous.as_deref() == Some(why.as_str()) {
            return Ok(());
        }
        let mut v = Doc::new();
        v.insert("posting_error".into(), json!(why));
        k.set_child_values_tx(ctx, &mut tx, "Asset", "schedule", row, v).await?;
        let managers: Vec<Uuid> = sqlx::query_scalar(
            "select u.id from app_user u join user_role r on r.tenant_id = u.tenant_id and r.user_id = u.id \
             where u.tenant_id = $1 and r.role = 'Asset Manager' and u.is_active",
        )
        .bind(ctx.tenant)
        .fetch_all(&mut *tx)
        .await?;
        let title = format!("Depreciation of asset {number} could not be booked");
        for user in managers {
            ferrum_kernel::notify::send(&mut tx, ctx.tenant, user, &title, Some(&why), Some("Asset"), Some(asset)).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    .await;
    if let Err(e) = done {
        tracing::warn!(tenant = %ctx.tenant, %asset, "recording a depreciation failure failed: {e}");
    }
}
