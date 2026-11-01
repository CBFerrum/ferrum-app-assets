//! Asset reports: the fixed asset register (cost, accumulated depreciation and net book value as of
//! a date, by asset, category or location), the depreciation schedule (planned vs booked), the
//! movement history, maintenance due and disposal gains and losses. Every list applies the
//! caller's record access, so a custodian sees only the assets in their custody.

use async_trait::async_trait;
use chrono::{Duration, NaiveDate};
use ferrum_domain::reports::{Params, ReportCompany, ReportSource, col, param_def, period, read_tx, resolve_company, today};
use ferrum_domain::util::money;
use ferrum_kernel::{Action, Ctx, Error, Kernel, Result, Tx, row_scope};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

const MAX_ROWS: i64 = 10_000;

pub struct AssetReports;

struct Def {
    name: &'static str,
    label: &'static str,
    entity: &'static str,
    params: &'static [&'static str],
}

const DEFS: &[Def] = &[
    Def {
        name: "fixed_asset_register",
        label: "Fixed Asset Register",
        entity: "Asset",
        params: &["company", "as_of", "asset_category", "location", "finance_book", "group_by"],
    },
    Def {
        name: "asset_depreciation_schedule",
        label: "Asset Depreciation Schedule",
        entity: "Asset",
        params: &["company", "from_date", "to_date", "asset", "asset_category", "finance_book"],
    },
    Def {
        name: "asset_movement_history",
        label: "Asset Movement History",
        entity: "AssetMovement",
        params: &["company", "from_date", "to_date", "asset", "location"],
    },
    Def { name: "asset_maintenance_due", label: "Asset Maintenance Due", entity: "AssetMaintenance", params: &["company", "as_of", "days", "assign_to"] },
    Def {
        name: "asset_disposal_gain_loss",
        label: "Asset Disposals: Gain and Loss",
        entity: "AssetDisposal",
        params: &["company", "from_date", "to_date", "disposal_type"],
    },
];

fn param(name: &str) -> Value {
    let link = |label: &str, to: &str| json!({"name": name, "label": label, "type": "link", "to": to});
    match name {
        "asset" => link("Asset", "Asset"),
        "asset_category" => link("Asset Category", "AssetCategory"),
        "location" => link("Location", "Location"),
        "finance_book" => link("Finance Book", "FinanceBook"),
        "group_by" => json!({"name": name, "label": "Group By", "type": "select", "options": ["Asset", "Asset Category", "Location"], "default": "Asset"}),
        "days" => json!({"name": name, "label": "Due Within (Days)", "type": "select", "options": ["0", "7", "30", "90", "365"], "default": "30"}),
        "assign_to" => json!({"name": name, "label": "Assigned To (email)", "type": "data"}),
        "disposal_type" => json!({"name": name, "label": "Disposal Type", "type": "select", "options": ["Sell", "Scrap"]}),
        other => param_def(other),
    }
}

#[async_trait]
impl ReportSource for AssetReports {
    fn catalog(&self) -> Vec<Value> {
        DEFS.iter()
            .map(|d| json!({"name": d.name, "label": d.label, "module": "assets", "entity": d.entity, "params": d.params.iter().map(|p| param(p)).collect::<Vec<_>>()}))
            .collect()
    }

    async fn run(&self, k: &Kernel, ctx: &Ctx, name: &str, raw: &HashMap<String, String>) -> Result<Value> {
        let p = Params { raw };
        let mut tx = read_tx(k, ctx).await?;
        let co = resolve_company(&mut tx, ctx, &p).await?;
        let mut out = match name {
            "fixed_asset_register" => register(k, ctx, &mut tx, &co, &p).await?,
            "asset_depreciation_schedule" => depreciation(k, ctx, &mut tx, &co, &p).await?,
            "asset_movement_history" => movements(k, ctx, &mut tx, &co, &p).await?,
            "asset_maintenance_due" => maintenance_due(k, ctx, &mut tx, &co, &p).await?,
            "asset_disposal_gain_loss" => disposals(k, ctx, &mut tx, &co, &p).await?,
            _ => return Err(Error::NotFound(format!("report {name}"))),
        };
        tx.rollback().await?;
        if let Value::Object(m) = &mut out {
            let meta = m.entry("meta").or_insert_with(|| json!({}));
            meta["company"] = json!(co.name);
            meta["currency"] = json!(co.currency);
        }
        Ok(out)
    }
}

/// Appends the caller's record access on `entity` for table alias `alias`.
fn scoped(k: &Kernel, ctx: &Ctx, qb: &mut QueryBuilder<Postgres>, entity: &str, alias: &str) -> Result<()> {
    let ent = k.entity(ctx, entity)?;
    row_scope(ctx, &ent, Action::Read)?.push_sql(qb, alias);
    Ok(())
}

#[derive(sqlx::FromRow)]
struct RegisterRow {
    id: Uuid,
    number: Option<String>,
    asset_name: Option<String>,
    category: Option<String>,
    location: Option<String>,
    custodian: Option<String>,
    status: Option<String>,
    purchase_date: Option<NaiveDate>,
    available_for_use_date: Option<NaiveDate>,
    cost: Decimal,
    accumulated: Decimal,
}

/// Assets in service on the date with their cost (purchase, capitalized repairs and value
/// adjustments up to the date), accumulated depreciation (opening and booked up to the date) and
/// net book value, at the location they were at on the date.
async fn register(k: &Kernel, ctx: &Ctx, tx: &mut Tx, co: &ReportCompany, p: &Params<'_>) -> Result<Value> {
    let as_of = p.date("as_of")?.unwrap_or_else(today);
    let (category, location, book) = (p.uuid("asset_category")?, p.uuid("location")?, p.uuid("finance_book")?);
    let group = match p.str("group_by") {
        Some("Asset Category") => "Asset Category",
        Some("Location") => "Location",
        _ => "Asset",
    };
    let moved = "from e_asset_movement__assets i join e_asset_movement m on m.tenant_id = i.tenant_id and m.id = i.parent_id \
                 where i.tenant_id = a.tenant_id and i.asset = a.id and m.docstatus = 1 and m.transaction_date";
    let mut qb = QueryBuilder::<Postgres>::new(
        "select x.id, x.number, x.asset_name, x.category, l.location_name as location, x.custodian, x.status, x.purchase_date, x.available_for_use_date, \
         x.cost, x.accumulated from (select a.id, a.number, a.asset_name, c.category_name as category, a.custodian, a.status, a.purchase_date, a.available_for_use_date, \
         coalesce((select i.target_location ",
    );
    qb.push(moved).push(" <= ").push_bind(as_of).push(" order by m.transaction_date desc, m.created_at desc limit 1), (select i.source_location ");
    qb.push(moved).push(" > ").push_bind(as_of).push(" order by m.transaction_date, m.created_at limit 1), a.location) as location_id, ");
    qb.push("a.gross_purchase_amount + coalesce((select sum(r.repair_cost) from e_asset_repair r where r.tenant_id = a.tenant_id and r.asset = a.id \
             and r.docstatus = 1 and r.capitalize_repair_cost and r.completion_date <= ")
        .push_bind(as_of)
        .push("), 0) + coalesce((select sum(v.difference_amount) from e_asset_value_adjustment v where v.tenant_id = a.tenant_id and v.asset = a.id and v.docstatus = 1 \
               and v.adjustment_date <= ")
        .push_bind(as_of)
        .push("), 0) as cost, coalesce(a.opening_accumulated_depreciation, 0) + coalesce((select sum(s.depreciation_amount) from e_asset__schedule s \
               where s.tenant_id = a.tenant_id and s.parent_id = a.id and s.booked and s.schedule_date <= ")
        .push_bind(as_of)
        .push("), 0) as accumulated from e_asset a left join e_asset_category c on c.tenant_id = a.tenant_id and c.id = a.asset_category where a.tenant_id = ")
        .push_bind(ctx.tenant)
        .push(" and a.company = ")
        .push_bind(co.id)
        .push(" and a.docstatus = 1 and a.purchase_date <= ")
        .push_bind(as_of)
        .push(" and not exists (select 1 from e_asset_disposal dd where dd.tenant_id = a.tenant_id and dd.asset = a.id and dd.docstatus = 1 and dd.disposal_date <= ")
        .push_bind(as_of)
        .push(")");
    if let Some(c) = category {
        qb.push(" and a.asset_category = ").push_bind(c);
    }
    if let Some(b) = book {
        qb.push(" and a.finance_book = ").push_bind(b);
    }
    scoped(k, ctx, &mut qb, "Asset", "a")?;
    qb.push(") x left join e_location l on l.tenant_id = ").push_bind(ctx.tenant).push(" and l.id = x.location_id");
    if let Some(l) = location {
        qb.push(" where x.location_id = ").push_bind(l);
    }
    qb.push(" order by x.category nulls last, x.number limit ").push_bind(MAX_ROWS + 1);
    let data: Vec<RegisterRow> = qb.build_query_as().fetch_all(&mut **tx).await?;
    let truncated = data.len() as i64 > MAX_ROWS;
    let (mut cost, mut acc) = (Decimal::ZERO, Decimal::ZERO);
    let mut groups: BTreeMap<String, (i64, Decimal, Decimal)> = BTreeMap::new();
    let mut rows = Vec::new();
    for r in data.into_iter().take(MAX_ROWS as usize) {
        cost += r.cost;
        acc += r.accumulated;
        let key = match group {
            "Asset Category" => r.category.clone().unwrap_or_default(),
            "Location" => r.location.clone().unwrap_or_default(),
            _ => {
                rows.push(json!({"asset": r.number, "asset_id": r.id, "asset_name": r.asset_name, "asset_category": r.category, "location": r.location,
                    "custodian": r.custodian, "status": r.status, "purchase_date": r.purchase_date, "available_for_use_date": r.available_for_use_date,
                    "cost": money(r.cost), "accumulated_depreciation": money(r.accumulated), "net_book_value": money(r.cost - r.accumulated)}));
                continue;
            }
        };
        let g = groups.entry(key).or_default();
        g.0 += 1;
        g.1 += r.cost;
        g.2 += r.accumulated;
    }
    let money_cols = [
        col("cost", "Cost", "currency"),
        col("accumulated_depreciation", "Accumulated Depreciation", "currency"),
        col("net_book_value", "Net Book Value", "currency"),
    ];
    let columns: Vec<Value> = if group == "Asset" {
        let mut c = vec![
            col("asset", "Asset", "data"),
            col("asset_name", "Asset Name", "data"),
            col("asset_category", "Category", "data"),
            col("location", "Location", "data"),
            col("custodian", "Custodian", "data"),
            col("available_for_use_date", "In Use From", "date"),
        ];
        c.extend(money_cols);
        c
    } else {
        let field = if group == "Location" { "location" } else { "asset_category" };
        rows = groups
            .into_iter()
            .map(|(name, (n, c, a))| json!({field: name, "assets": n, "cost": money(c), "accumulated_depreciation": money(a), "net_book_value": money(c - a)}))
            .collect();
        let mut c = vec![col(field, group, "data"), col("assets", "Assets", "int")];
        c.extend(money_cols);
        c
    };
    Ok(json!({
        "columns": columns,
        "rows": rows,
        "totals": {"cost": money(cost), "accumulated_depreciation": money(acc), "net_book_value": money(cost - acc)},
        "meta": {"as_of": as_of, "group_by": group, "truncated": truncated},
    }))
}

#[derive(sqlx::FromRow)]
struct ScheduleRow {
    asset_id: Uuid,
    asset: Option<String>,
    asset_name: Option<String>,
    category: Option<String>,
    schedule_date: NaiveDate,
    amount: Decimal,
    accumulated: Option<Decimal>,
    booked: bool,
    journal_entry: Option<String>,
    at_disposal: bool,
    posting_error: Option<String>,
}

/// Every schedule row in the period: planned against booked, with what is overdue.
async fn depreciation(k: &Kernel, ctx: &Ctx, tx: &mut Tx, co: &ReportCompany, p: &Params<'_>) -> Result<Value> {
    let (from, to) = period(p, co)?;
    let mut qb = QueryBuilder::<Postgres>::new(
        "select a.id as asset_id, a.number as asset, a.asset_name, c.category_name as category, s.schedule_date, s.depreciation_amount as amount, \
         s.accumulated_depreciation as accumulated, s.booked, je.number as journal_entry, s.disposal is not null as at_disposal, s.posting_error \
         from e_asset__schedule s join e_asset a on a.tenant_id = s.tenant_id and a.id = s.parent_id \
         left join e_asset_category c on c.tenant_id = a.tenant_id and c.id = a.asset_category \
         left join e_journal_entry je on je.tenant_id = s.tenant_id and je.id = s.journal_entry where s.tenant_id = ",
    );
    qb.push_bind(ctx.tenant).push(" and a.company = ").push_bind(co.id).push(" and a.docstatus = 1 and s.schedule_date between ").push_bind(from);
    qb.push(" and ").push_bind(to);
    if let Some(a) = p.uuid("asset")? {
        qb.push(" and a.id = ").push_bind(a);
    }
    if let Some(c) = p.uuid("asset_category")? {
        qb.push(" and a.asset_category = ").push_bind(c);
    }
    if let Some(b) = p.uuid("finance_book")? {
        qb.push(" and a.finance_book = ").push_bind(b);
    }
    scoped(k, ctx, &mut qb, "Asset", "a")?;
    qb.push(" order by s.schedule_date, a.number, s.idx limit ").push_bind(MAX_ROWS + 1);
    let data: Vec<ScheduleRow> = qb.build_query_as().fetch_all(&mut **tx).await?;
    let truncated = data.len() as i64 > MAX_ROWS;
    let now = today();
    let (mut planned, mut booked, mut overdue) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    let rows: Vec<Value> = data
        .into_iter()
        .take(MAX_ROWS as usize)
        .map(|r| {
            planned += r.amount;
            let status = if r.booked {
                booked += r.amount;
                if r.at_disposal { "Booked at Disposal" } else { "Booked" }
            } else if r.schedule_date <= now {
                overdue += r.amount;
                "Due"
            } else {
                "Planned"
            };
            json!({"asset": r.asset, "asset_id": r.asset_id, "asset_name": r.asset_name, "asset_category": r.category, "schedule_date": r.schedule_date,
                "planned_amount": money(r.amount), "booked_amount": money(if r.booked { r.amount } else { Decimal::ZERO }),
                "accumulated_depreciation": money(r.accumulated.unwrap_or_default()), "journal_entry": r.journal_entry, "status": status, "posting_error": r.posting_error})
        })
        .collect();
    Ok(json!({
        "columns": [col("schedule_date", "Date", "date"), col("asset", "Asset", "data"), col("asset_name", "Asset Name", "data"),
            col("asset_category", "Category", "data"), col("planned_amount", "Planned", "currency"), col("booked_amount", "Booked", "currency"),
            col("accumulated_depreciation", "Accumulated", "currency"), col("journal_entry", "Journal Entry", "data"), col("status", "Status", "data")],
        "rows": rows,
        "totals": {"planned_amount": money(planned), "booked_amount": money(booked), "not_booked": money(planned - booked), "overdue": money(overdue)},
        "meta": {"from_date": from, "to_date": to, "truncated": truncated},
    }))
}

#[derive(sqlx::FromRow)]
struct MoveRow {
    movement_id: Uuid,
    movement: Option<String>,
    transaction_date: NaiveDate,
    purpose: Option<String>,
    asset: Option<String>,
    asset_name: Option<String>,
    from_location: Option<String>,
    to_location: Option<String>,
    from_custodian: Option<String>,
    to_custodian: Option<String>,
    due_back: Option<NaiveDate>,
}

/// Submitted movements, oldest first: where each asset went, who held it, and when it was due back.
async fn movements(k: &Kernel, ctx: &Ctx, tx: &mut Tx, co: &ReportCompany, p: &Params<'_>) -> Result<Value> {
    let (from, to) = period(p, co)?;
    let mut qb = QueryBuilder::<Postgres>::new(
        "select m.id as movement_id, m.number as movement, m.transaction_date, m.purpose, a.number as asset, a.asset_name, \
         ls.location_name as from_location, lt.location_name as to_location, i.from_custodian, i.to_custodian, i.expected_return_date as due_back \
         from e_asset_movement__assets i join e_asset_movement m on m.tenant_id = i.tenant_id and m.id = i.parent_id \
         join e_asset a on a.tenant_id = i.tenant_id and a.id = i.asset \
         left join e_location ls on ls.tenant_id = i.tenant_id and ls.id = i.source_location \
         left join e_location lt on lt.tenant_id = i.tenant_id and lt.id = i.target_location where i.tenant_id = ",
    );
    qb.push_bind(ctx.tenant).push(" and m.company = ").push_bind(co.id).push(" and m.docstatus = 1 and m.transaction_date between ").push_bind(from);
    qb.push(" and ").push_bind(to);
    if let Some(a) = p.uuid("asset")? {
        qb.push(" and i.asset = ").push_bind(a);
    }
    if let Some(l) = p.uuid("location")? {
        qb.push(" and (i.source_location = ").push_bind(l).push(" or i.target_location = ").push_bind(l).push(")");
    }
    scoped(k, ctx, &mut qb, "AssetMovement", "m")?;
    qb.push(" order by m.transaction_date, m.created_at, i.idx limit ").push_bind(MAX_ROWS + 1);
    let data: Vec<MoveRow> = qb.build_query_as().fetch_all(&mut **tx).await?;
    let truncated = data.len() as i64 > MAX_ROWS;
    let rows: Vec<Value> = data
        .into_iter()
        .take(MAX_ROWS as usize)
        .map(|r| {
            json!({"transaction_date": r.transaction_date, "movement": r.movement, "movement_id": r.movement_id, "purpose": r.purpose, "asset": r.asset,
                "asset_name": r.asset_name, "from_location": r.from_location, "to_location": r.to_location, "from_custodian": r.from_custodian, "to_custodian": r.to_custodian,
                "due_back": r.due_back})
        })
        .collect();
    let count = rows.len();
    Ok(json!({
        "columns": [col("transaction_date", "Date", "date"), col("movement", "Movement", "data"), col("purpose", "Purpose", "data"), col("asset", "Asset", "data"),
            col("asset_name", "Asset Name", "data"), col("from_location", "From", "data"), col("to_location", "To", "data"),
            col("from_custodian", "From Custodian", "data"), col("to_custodian", "To Custodian", "data"), col("due_back", "Due Back", "date")],
        "rows": rows,
        "totals": {"count": count},
        "meta": {"from_date": from, "to_date": to, "truncated": truncated},
    }))
}

#[derive(sqlx::FromRow)]
struct DueRow {
    plan_id: Uuid,
    asset_id: Uuid,
    asset: Option<String>,
    asset_name: Option<String>,
    task_name: Option<String>,
    maintenance_type: Option<String>,
    periodicity: Option<String>,
    assign_to: Option<String>,
    last_completion_date: Option<NaiveDate>,
    next_due_date: NaiveDate,
}

/// Maintenance tasks due by the date (plus a look-ahead), overdue first.
async fn maintenance_due(k: &Kernel, ctx: &Ctx, tx: &mut Tx, co: &ReportCompany, p: &Params<'_>) -> Result<Value> {
    let as_of = p.date("as_of")?.unwrap_or_else(today);
    let days: i64 = p.str("days").map(|d| d.parse().map_err(|_| Error::field("days", "must be a whole number"))).transpose()?.unwrap_or(30).clamp(0, 3660);
    let horizon = as_of + Duration::days(days);
    let mut qb = QueryBuilder::<Postgres>::new(
        "select m.id as plan_id, a.id as asset_id, a.number as asset, a.asset_name, t.task_name, t.maintenance_type, t.periodicity, t.assign_to, t.last_completion_date, t.next_due_date \
         from e_asset_maintenance__tasks t join e_asset_maintenance m on m.tenant_id = t.tenant_id and m.id = t.parent_id \
         join e_asset a on a.tenant_id = m.tenant_id and a.id = m.asset where t.tenant_id = ",
    );
    qb.push_bind(ctx.tenant)
        .push(" and m.company = ")
        .push_bind(co.id)
        .push(" and a.docstatus = 1 and a.disposal is null and t.next_due_date <= ")
        .push_bind(horizon);
    if let Some(u) = p.str("assign_to") {
        qb.push(" and lower(t.assign_to) = ").push_bind(u.to_lowercase());
    }
    scoped(k, ctx, &mut qb, "AssetMaintenance", "m")?;
    qb.push(" order by t.next_due_date, a.number limit ").push_bind(MAX_ROWS + 1);
    let data: Vec<DueRow> = qb.build_query_as().fetch_all(&mut **tx).await?;
    let truncated = data.len() as i64 > MAX_ROWS;
    let mut overdue = 0;
    let rows: Vec<Value> = data
        .into_iter()
        .take(MAX_ROWS as usize)
        .map(|r| {
            let late = (as_of - r.next_due_date).num_days();
            let status = match late {
                l if l > 0 => {
                    overdue += 1;
                    "Overdue"
                }
                0 => "Due Today",
                _ => "Upcoming",
            };
            json!({"asset": r.asset, "asset_id": r.asset_id, "asset_name": r.asset_name, "asset_maintenance": r.plan_id, "task_name": r.task_name, "maintenance_type": r.maintenance_type,
                "periodicity": r.periodicity, "assign_to": r.assign_to, "last_completion_date": r.last_completion_date, "next_due_date": r.next_due_date,
                "days_overdue": late.max(0), "status": status})
        })
        .collect();
    let count = rows.len();
    Ok(json!({
        "columns": [col("next_due_date", "Due", "date"), col("status", "Status", "data"), col("days_overdue", "Days Overdue", "int"), col("asset", "Asset", "data"),
            col("asset_name", "Asset Name", "data"), col("task_name", "Task", "data"), col("maintenance_type", "Type", "data"),
            col("periodicity", "Periodicity", "data"), col("assign_to", "Assigned To", "data"), col("last_completion_date", "Last Done", "date")],
        "rows": rows,
        "totals": {"count": count, "overdue": overdue},
        "meta": {"as_of": as_of, "due_by": horizon, "truncated": truncated},
    }))
}

#[derive(sqlx::FromRow)]
struct DisposalRow {
    disposal_id: Uuid,
    disposal: Option<String>,
    disposal_date: NaiveDate,
    disposal_type: Option<String>,
    asset: Option<String>,
    asset_name: Option<String>,
    customer: Option<String>,
    sales_invoice: Option<String>,
    gross_amount: Option<Decimal>,
    accumulated_depreciation: Option<Decimal>,
    net_book_value: Option<Decimal>,
    proceeds: Option<Decimal>,
    gain_or_loss: Option<Decimal>,
}

/// Submitted disposals in the period with what each asset was carried at and what it fetched.
async fn disposals(k: &Kernel, ctx: &Ctx, tx: &mut Tx, co: &ReportCompany, p: &Params<'_>) -> Result<Value> {
    let (from, to) = period(p, co)?;
    let mut qb = QueryBuilder::<Postgres>::new(
        "select d.id as disposal_id, d.number as disposal, d.disposal_date, d.disposal_type, a.number as asset, a.asset_name, cu.customer_name as customer, \
         si.number as sales_invoice, d.gross_amount, d.accumulated_depreciation, d.net_book_value, d.proceeds, d.gain_or_loss \
         from e_asset_disposal d join e_asset a on a.tenant_id = d.tenant_id and a.id = d.asset \
         left join e_customer cu on cu.tenant_id = d.tenant_id and cu.id = d.customer \
         left join e_sales_invoice si on si.tenant_id = d.tenant_id and si.id = d.sales_invoice where d.tenant_id = ",
    );
    qb.push_bind(ctx.tenant).push(" and d.company = ").push_bind(co.id).push(" and d.docstatus = 1 and d.disposal_date between ").push_bind(from);
    qb.push(" and ").push_bind(to);
    if let Some(t) = p.str("disposal_type") {
        qb.push(" and d.disposal_type = ").push_bind(t.to_string());
    }
    scoped(k, ctx, &mut qb, "AssetDisposal", "d")?;
    qb.push(" order by d.disposal_date, d.number limit ").push_bind(MAX_ROWS + 1);
    let data: Vec<DisposalRow> = qb.build_query_as().fetch_all(&mut **tx).await?;
    let truncated = data.len() as i64 > MAX_ROWS;
    let mut t = [Decimal::ZERO; 7];
    let rows: Vec<Value> = data
        .into_iter()
        .take(MAX_ROWS as usize)
        .map(|r| {
            let g = r.gain_or_loss.unwrap_or_default();
            let vals = [r.gross_amount, r.accumulated_depreciation, r.net_book_value, r.proceeds].map(Option::unwrap_or_default);
            for (x, v) in t.iter_mut().zip(vals) {
                *x += v;
            }
            t[4] += g;
            if g > Decimal::ZERO {
                t[5] += g;
            } else {
                t[6] -= g;
            }
            json!({"disposal": r.disposal, "disposal_id": r.disposal_id, "disposal_date": r.disposal_date, "disposal_type": r.disposal_type, "asset": r.asset,
                "asset_name": r.asset_name, "customer": r.customer, "sales_invoice": r.sales_invoice, "gross_amount": money(vals[0]),
                "accumulated_depreciation": money(vals[1]), "net_book_value": money(vals[2]), "proceeds": money(vals[3]), "gain_or_loss": money(g)})
        })
        .collect();
    Ok(json!({
        "columns": [col("disposal_date", "Date", "date"), col("disposal", "Disposal", "data"), col("disposal_type", "Type", "data"), col("asset", "Asset", "data"),
            col("asset_name", "Asset Name", "data"), col("customer", "Customer", "data"), col("sales_invoice", "Sales Invoice", "data"),
            col("gross_amount", "Cost", "currency"), col("accumulated_depreciation", "Accumulated Depreciation", "currency"),
            col("net_book_value", "Net Book Value", "currency"), col("proceeds", "Proceeds", "currency"), col("gain_or_loss", "Gain / (Loss)", "currency")],
        "rows": rows,
        "totals": {"gross_amount": money(t[0]), "accumulated_depreciation": money(t[1]), "net_book_value": money(t[2]), "proceeds": money(t[3]),
            "gain_or_loss": money(t[4]), "gains": money(t[5]), "losses": money(t[6])},
        "meta": {"from_date": from, "to_date": to, "truncated": truncated},
    }))
}
