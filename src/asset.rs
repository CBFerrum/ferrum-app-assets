//! The asset engine: defaults and validation, the depreciation schedule (generated, booked,
//! re-planned, cut at a disposal), capitalization out of CWIP, and the figures and status, which
//! are recomputed from the schedule and the documents behind them on every change.
//!
//! Engine changes load the submitted asset with a row lock (`open`), change it in memory and
//! write the difference back (`save`), so every change to one asset is serialized.

use crate::schedule::{self, Method, Plan};
use crate::setup;
use crate::{check_places, installed, int, named};
use async_trait::async_trait;
use chrono::NaiveDate;
use ferrum_domain::ledger::{self, Entry, Line};
use ferrum_domain::util::{b, company, d, elevated, get, id, money, rows, s, set, sys};
use ferrum_kernel::value::parse_date;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, FieldError, HookCtx, Kernel, Result, Tx, store};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;

/// A date field (`None` when empty or not a date).
pub(crate) fn day(doc: &Doc, k: &str) -> Option<NaiveDate> {
    s(doc, k).and_then(|v| parse_date(&v[..v.len().min(10)]).ok())
}

fn obj(v: Value) -> Doc {
    match v {
        Value::Object(m) => m,
        _ => Doc::new(),
    }
}

/// The asset's depreciation plan from its fields; the errors name the fields to fix.
pub(crate) fn plan_of(doc: &Doc, precision: u32) -> std::result::Result<Plan, Vec<FieldError>> {
    let mut errors = Vec::new();
    let method = s(doc, "depreciation_method").and_then(Method::parse);
    if method.is_none() {
        errors.push(FieldError::new("depreciation_method", "is required"));
    }
    let periods = int(doc, "total_number_of_depreciations");
    let frequency = int(doc, "frequency_of_depreciation");
    if periods < 1 {
        errors.push(FieldError::new("total_number_of_depreciations", "must be at least 1"));
    }
    if frequency < 1 {
        errors.push(FieldError::new("frequency_of_depreciation", "must be at least 1 month"));
    }
    if periods * frequency > 1200 {
        errors.push(FieldError::new("total_number_of_depreciations", "the useful life cannot exceed 100 years"));
    }
    let rate = d(doc, "rate_of_depreciation");
    if method == Some(Method::WrittenDownValue) && rate <= Decimal::ZERO {
        errors.push(FieldError::new("rate_of_depreciation", "written down value needs a rate"));
    }
    let afu = day(doc, "available_for_use_date");
    let start = day(doc, "depreciation_start_date");
    if afu.is_none() {
        errors.push(FieldError::new("available_for_use_date", "is required"));
    }
    match (start, afu) {
        (None, _) if method != Some(Method::Manual) => errors.push(FieldError::new("depreciation_start_date", "is required")),
        (Some(st), Some(a)) if st < a => errors.push(FieldError::new("depreciation_start_date", "cannot be before the available-for-use date")),
        _ => {}
    }
    let opening_booked = int(doc, "opening_number_of_booked_depreciations");
    if opening_booked < 0 || (periods >= 1 && opening_booked > periods) {
        errors.push(FieldError::new("opening_number_of_booked_depreciations", "cannot exceed the total number of depreciations"));
    }
    let (Some(method), Some(afu), true) = (method, afu, errors.is_empty()) else { return Err(errors) };
    Ok(Plan {
        method,
        periods: periods as u32,
        frequency: frequency as u32,
        annual_rate: rate,
        pro_rata: s(doc, "start_convention") != Some("Full First Period"),
        available_for_use: afu,
        start: start.unwrap_or(afu),
        opening_booked: opening_booked as u32,
        salvage: d(doc, "salvage_value"),
        precision,
    })
}

fn schedule_rows(doc: &Doc) -> Vec<Doc> {
    let mut list = rows(doc, "schedule");
    // By date, booked rows first: booked rows are always the oldest.
    list.sort_by_key(|r| (s(r, "schedule_date").unwrap_or_default().to_string(), !b(r, "booked")));
    list
}

fn put_rows(doc: &mut Doc, list: Vec<Doc>) {
    set(doc, "schedule", Value::Array(list.into_iter().map(Value::Object).collect()));
}

fn new_row(on: NaiveDate, amount: Decimal) -> Doc {
    obj(json!({"id": Uuid::now_v7().to_string(), "schedule_date": on.to_string(), "depreciation_amount": money(amount), "booked": false}))
}

fn row_day(r: &Doc) -> NaiveDate {
    day(r, "schedule_date").unwrap_or(NaiveDate::MIN)
}

/// Capitalized repairs and value adjustments submitted for an asset.
async fn changes(tx: &mut Tx, tenant: Uuid, asset: Uuid) -> Result<(Decimal, Decimal)> {
    Ok(sqlx::query_as(
        "select coalesce((select sum(repair_cost) from e_asset_repair where tenant_id = $1 and asset = $2 and docstatus = 1 and capitalize_repair_cost), 0), \
         coalesce((select sum(difference_amount) from e_asset_value_adjustment where tenant_id = $1 and asset = $2 and docstatus = 1), 0)",
    )
    .bind(tenant)
    .bind(asset)
    .fetch_one(&mut **tx)
    .await?)
}

/// Fills the schedule's running total and order, and the asset's figures from it.
fn columns(doc: &mut Doc, repairs: Decimal, adjustments: Decimal) {
    let opening = d(doc, "opening_accumulated_depreciation");
    let mut list = schedule_rows(doc);
    let (mut running, mut booked) = (opening, opening);
    let mut next = Value::Null;
    for (i, r) in list.iter_mut().enumerate() {
        let a = d(r, "depreciation_amount");
        running += a;
        set(r, "idx", i);
        set(r, "accumulated_depreciation", money(running));
        if b(r, "booked") {
            booked += a;
        } else if next.is_null() {
            next = r.get("schedule_date").cloned().unwrap_or(Value::Null);
        }
    }
    put_rows(doc, list);
    let cost = d(doc, "gross_purchase_amount") + repairs + adjustments;
    set(doc, "additional_asset_cost", money(repairs));
    set(doc, "value_adjustments", money(adjustments));
    set(doc, "total_asset_cost", money(cost));
    set(doc, "accumulated_depreciation", money(booked));
    set(doc, "value_after_depreciation", money(cost - booked));
    set(doc, "next_depreciation_date", next);
}

/// The status a submitted asset is in (`exclude`: a document about to be deleted).
async fn status(tx: &mut Tx, tenant: Uuid, doc: &Doc, exclude: Option<Uuid>) -> Result<&'static str> {
    match doc.get("docstatus").and_then(Value::as_i64) {
        Some(1) => {}
        Some(2) => return Ok("Cancelled"),
        _ => return Ok("Draft"),
    }
    let asset = id(doc, "id").ok_or_else(|| Error::internal("asset without id"))?;
    if let Some(disposal) = id(doc, "disposal") {
        let kind: Option<String> = sqlx::query_scalar("select disposal_type from e_asset_disposal where tenant_id = $1 and id = $2")
            .bind(tenant)
            .bind(disposal)
            .fetch_optional(&mut **tx)
            .await?;
        return Ok(if kind.as_deref() == Some("Sell") { "Sold" } else { "Scrapped" });
    }
    let (repair, servicing): (bool, bool) = sqlx::query_as(
        "select exists(select 1 from e_asset_repair where tenant_id = $1 and asset = $2 and docstatus = 0 and repair_status = 'Pending' and id is distinct from $3), \
         exists(select 1 from e_asset_maintenance_log where tenant_id = $1 and asset = $2 and docstatus = 0 and maintenance_status = 'In Progress' and id is distinct from $3)",
    )
    .bind(tenant)
    .bind(asset)
    .bind(exclude)
    .fetch_one(&mut **tx)
    .await?;
    let value = d(doc, "value_after_depreciation");
    Ok(if repair {
        "Out of Order"
    } else if servicing {
        "In Maintenance"
    } else if b(doc, "calculate_depreciation") && value <= d(doc, "salvage_value") {
        "Fully Depreciated"
    } else if d(doc, "accumulated_depreciation") > Decimal::ZERO {
        "Partially Depreciated"
    } else {
        "Submitted"
    })
}

/// Loads a submitted asset for an engine change, locked for the rest of the transaction.
/// Returns the document to change and the stored version to diff against.
pub(crate) async fn open(k: &Kernel, ctx: &Ctx, tx: &mut Tx, asset: Uuid) -> Result<(Doc, Doc)> {
    let doc = k.get_tx(&sys(ctx), tx, "Asset", asset, true).await?;
    Ok((doc.clone(), doc))
}

/// Recomputes the figures and status of a changed asset and writes the difference.
pub(crate) async fn save(k: &Kernel, ctx: &Ctx, tx: &mut Tx, mut doc: Doc, old: &Doc, exclude: Option<Uuid>) -> Result<Doc> {
    let asset = id(&doc, "id").ok_or_else(|| Error::internal("asset without id"))?;
    let (repairs, adjustments) = changes(tx, ctx.tenant, asset).await?;
    columns(&mut doc, repairs, adjustments);
    let st = status(tx, ctx.tenant, &doc, exclude).await?;
    set(&mut doc, "status", st);
    let ent = k.entity(ctx, "Asset")?;
    store::update_row(tx, &ctx.schema, &ent, ctx.tenant, &doc, old).await?;
    Ok(doc)
}

/// Recomputes and stores an asset's figures and status.
pub(crate) async fn refresh(k: &Kernel, ctx: &Ctx, tx: &mut Tx, asset: Uuid, exclude: Option<Uuid>) -> Result<()> {
    let (doc, old) = open(k, ctx, tx, asset).await?;
    save(k, ctx, tx, doc, &old, exclude).await?;
    Ok(())
}

/// Net book value now: cost (with capitalized repairs and value adjustments) less opening and
/// booked depreciation, counting rows booked in memory.
pub(crate) async fn net_book_value(tx: &mut Tx, ctx: &Ctx, doc: &Doc) -> Result<Decimal> {
    let asset = id(doc, "id").ok_or_else(|| Error::internal("asset without id"))?;
    let (repairs, adjustments) = changes(tx, ctx.tenant, asset).await?;
    let booked: Decimal = rows(doc, "schedule").iter().filter(|r| b(r, "booked")).map(|r| d(r, "depreciation_amount")).sum();
    Ok(d(doc, "gross_purchase_amount") + repairs + adjustments - d(doc, "opening_accumulated_depreciation") - booked)
}

/// The latest booked depreciation date.
pub(crate) fn booked_until(doc: &Doc) -> Option<NaiveDate> {
    rows(doc, "schedule").iter().filter(|r| b(r, "booked")).map(row_day).max()
}

/// Refuses a change to an asset that is not in service (draft, cancelled, sold or scrapped).
pub(crate) fn in_service(asset: &Doc) -> Result<()> {
    let name = named(asset);
    match asset.get("docstatus").and_then(Value::as_i64) {
        Some(1) if id(asset, "disposal").is_none() => Ok(()),
        Some(1) => Err(Error::field("asset", format!("asset {name} was {}", s(asset, "status").unwrap_or("disposed of").to_lowercase()))),
        Some(0) => Err(Error::field("asset", format!("submit asset {name} first"))),
        _ => Err(Error::field("asset", format!("asset {name} is cancelled"))),
    }
}

/// Books schedule row `i` as a submitted depreciation journal entry: debit depreciation
/// expense, credit accumulated depreciation, on the asset's cost center and the row's date.
async fn book(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &mut Doc, i: usize) -> Result<()> {
    let mut list = schedule_rows(doc);
    let amount = d(&list[i], "depreciation_amount");
    let on = row_day(&list[i]);
    let expense = id(doc, "depreciation_expense_account").ok_or_else(|| Error::invalid("the asset has no depreciation expense account"))?;
    let accumulated = id(doc, "accumulated_depreciation_account").ok_or_else(|| Error::invalid("the asset has no accumulated depreciation account"))?;
    let cc = id(doc, "cost_center").map(|c| c.to_string());
    let what = format!("Depreciation of {} {}", named(doc), s(doc, "asset_name").unwrap_or_default());
    let input = json!({
        "company": doc.get("company"), "posting_date": on.to_string(), "entry_type": "Depreciation", "fa_asset": doc.get("id"), "remark": what,
        "accounts": [
            {"account": expense.to_string(), "debit": money(amount), "cost_center": cc, "remarks": what},
            {"account": accumulated.to_string(), "credit": money(amount), "cost_center": cc, "remarks": what},
        ],
    });
    let engine = elevated(ctx);
    let je = k.insert_tx(&engine, tx, "JournalEntry", obj(input)).await?;
    let je = id(&je, "id").ok_or_else(|| Error::internal("journal entry without id"))?;
    k.submit_tx(&engine, tx, "JournalEntry", je, None).await?;
    set(&mut list[i], "journal_entry", je.to_string());
    set(&mut list[i], "booked", true);
    set(&mut list[i], "posting_error", Value::Null);
    put_rows(doc, list);
    Ok(())
}

/// Unbooks row `i`: the row lets go of its journal entry first (the journal entry guard refuses
/// to cancel an entry a schedule row still books), then the entry is cancelled (reversed).
async fn unbook(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &mut Doc, i: usize) -> Result<()> {
    let mut list = schedule_rows(doc);
    if let Some(row) = id(&list[i], "id") {
        let mut v = Doc::new();
        v.insert("journal_entry".into(), Value::Null);
        v.insert("booked".into(), json!(false));
        match k.set_child_values_tx(&sys(ctx), tx, "Asset", "schedule", row, v).await {
            Ok(()) | Err(Error::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    if let Some(je) = id(&list[i], "journal_entry") {
        k.cancel_tx(&elevated(ctx), tx, "JournalEntry", je, None).await?;
    }
    set(&mut list[i], "journal_entry", Value::Null);
    set(&mut list[i], "booked", false);
    put_rows(doc, list);
    Ok(())
}

/// Books the next unbooked row if it is due by `until`; `false` when there is none.
pub(crate) async fn book_next(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &mut Doc, until: NaiveDate) -> Result<bool> {
    let list = schedule_rows(doc);
    match list.iter().position(|r| !b(r, "booked")) {
        Some(i) if row_day(&list[i]) <= until => {
            book(k, ctx, tx, doc, i).await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Books every unbooked row dated on or before `until`, oldest first.
pub(crate) async fn book_until(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &mut Doc, until: NaiveDate) -> Result<usize> {
    let mut n = 0;
    while book_next(k, ctx, tx, doc, until).await? {
        n += 1;
    }
    Ok(n)
}

/// Re-plans the unbooked rows after the asset's value changed by `change` (the document making
/// the change is already counted in the asset's figures). Generated methods recompute the periods
/// left from the net book value; a manual plan is rescaled in proportion to the value it
/// depreciates. Never below salvage.
pub(crate) async fn replan(tx: &mut Tx, ctx: &Ctx, doc: &mut Doc, change: Decimal, precision: u32) -> Result<()> {
    if !b(doc, "calculate_depreciation") {
        return Ok(());
    }
    let plan = plan_of(doc, precision).map_err(Error::Validation)?;
    let value = net_book_value(tx, ctx, doc).await?;
    if value < plan.salvage {
        return Err(Error::invalid(format!(
            "this would take asset {}'s net book value ({}) below its salvage value ({})",
            named(doc),
            value.normalize(),
            plan.salvage.normalize()
        )));
    }
    let (booked, planned): (Vec<Doc>, Vec<Doc>) = schedule_rows(doc).into_iter().partition(|r| b(r, "booked"));
    let fresh: Vec<(NaiveDate, Decimal)> = if plan.method == Method::Manual {
        let amounts: Vec<Decimal> = planned.iter().map(|r| d(r, "depreciation_amount")).collect();
        let (before, after) = (value - change - plan.salvage, value - plan.salvage);
        let scaled = schedule::rescale(&amounts, before, after, precision).ok_or_else(|| {
            Error::invalid(format!("asset {} has no depreciation planned to spread this increase over; extend its manual schedule first", named(doc)))
        })?;
        planned.iter().map(row_day).zip(scaled).filter(|(_, a)| *a > Decimal::ZERO).collect()
    } else {
        let last = booked.iter().filter(|r| id(r, "disposal").is_none()).map(row_day).max();
        let slots: Vec<schedule::Slot> = plan.slots().into_iter().filter(|s| last.is_none_or(|l| s.date > l)).collect();
        if slots.is_empty() && change > Decimal::ZERO {
            return Err(Error::invalid(format!("asset {} has no depreciation periods left to spread this increase over", named(doc))));
        }
        plan.amounts(&slots, value)
    };
    let mut list = booked;
    list.extend(fresh.into_iter().map(|(on, a)| new_row(on, a)));
    put_rows(doc, list);
    Ok(())
}

/// Books depreciation up to a disposal date: rows booked after it are unbooked, rows due by it
/// are booked, the running period is depreciated for its days up to the date (a row of its own,
/// marked with the disposal) and the plan after it is removed. Returns the removed rows, which a
/// cancelled disposal puts back.
pub(crate) async fn depreciate_to_disposal(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &mut Doc, on: NaiveDate, disposal: Uuid, precision: u32) -> Result<Value> {
    while let Some(i) = schedule_rows(doc).iter().rposition(|r| b(r, "booked") && row_day(r) > on) {
        unbook(k, ctx, tx, doc, i).await?;
    }
    book_until(k, ctx, tx, doc, on).await?;
    let mut list = schedule_rows(doc);
    let Some(i) = list.iter().position(|r| !b(r, "booked")) else { return Ok(json!([])) };
    let removed = list.split_off(i);
    let snapshot: Vec<Value> =
        removed.iter().map(|r| json!({"schedule_date": r.get("schedule_date"), "depreciation_amount": r.get("depreciation_amount")})).collect();
    let plan = plan_of(doc, precision).map_err(Error::Validation)?;
    let next = &removed[0];
    let start = plan.period_start(row_day(next), list.last().map(row_day));
    let amount = schedule::partial(d(next, "depreciation_amount"), start, row_day(next), on, precision);
    if amount > Decimal::ZERO {
        let mut row = new_row(on, amount);
        set(&mut row, "disposal", disposal.to_string());
        list.push(row);
    }
    put_rows(doc, list);
    if let Some(i) = schedule_rows(doc).iter().position(|r| id(r, "disposal") == Some(disposal)) {
        book(k, ctx, tx, doc, i).await?;
    }
    Ok(Value::Array(snapshot))
}

/// Undoes `depreciate_to_disposal`: the disposal's own row is unbooked and removed, and the plan
/// it removed comes back (rows it had to unbook come back unbooked, to be booked again).
pub(crate) async fn restore_after_disposal(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &mut Doc, disposal: Uuid, snapshot: &Value) -> Result<()> {
    if let Some(i) = schedule_rows(doc).iter().position(|r| id(r, "disposal") == Some(disposal)) {
        unbook(k, ctx, tx, doc, i).await?;
        let mut list = schedule_rows(doc);
        list.remove(i);
        put_rows(doc, list);
    }
    let mut list = schedule_rows(doc);
    for r in snapshot.as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(Value::as_object) {
        if let Some(on) = day(r, "schedule_date") {
            list.push(new_row(on, d(r, "depreciation_amount")));
        }
    }
    put_rows(doc, list);
    Ok(())
}

/// Fills what the user left empty: name and category from the item, depreciation settings from
/// the category, the available-for-use and first depreciation dates, the cost center.
async fn defaults(hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
    if let Some(item) = id(doc, "item") {
        let it = get(hc.kernel, hc.ctx, hc.tx, "Item", item).await?;
        if s(doc, "asset_name").is_none() {
            set(doc, "asset_name", it.get("item_name").cloned().unwrap_or(Value::Null));
        }
        if id(doc, "asset_category").is_none()
            && let Some(c) = id(&it, "fa_asset_category")
        {
            set(doc, "asset_category", c.to_string());
        }
    }
    if b(doc, "calculate_depreciation")
        && let Some(cat) = id(doc, "asset_category")
    {
        let c = get(hc.kernel, hc.ctx, hc.tx, "AssetCategory", cat).await?;
        for f in ["depreciation_method", "total_number_of_depreciations", "frequency_of_depreciation", "rate_of_depreciation", "start_convention"] {
            if doc.get(f).is_none_or(Value::is_null) {
                set(doc, f, c.get(f).cloned().unwrap_or(Value::Null));
            }
        }
    }
    if s(doc, "available_for_use_date").is_none() {
        set(doc, "available_for_use_date", doc.get("purchase_date").cloned().unwrap_or(Value::Null));
    }
    if !b(doc, "is_existing_asset") {
        set(doc, "opening_accumulated_depreciation", money(Decimal::ZERO));
        set(doc, "opening_number_of_booked_depreciations", 0);
    }
    let frequency = int(doc, "frequency_of_depreciation");
    // Empty, or left before an available-for-use date that moved later: the default again.
    if b(doc, "calculate_depreciation")
        && frequency > 0
        && let Some(afu) = day(doc, "available_for_use_date")
        && day(doc, "depreciation_start_date").is_none_or(|start| start < afu)
    {
        // The end of the first period; for an existing asset, of the first period still to come.
        let behind = int(doc, "opening_number_of_booked_depreciations").clamp(0, 1200);
        let first = schedule::add_months(schedule::default_start(afu, frequency as u32), (behind * frequency) as i32);
        set(doc, "depreciation_start_date", first.to_string());
    }
    if id(doc, "cost_center").is_none()
        && let Some(cid) = id(doc, "company")
    {
        let co = get(hc.kernel, hc.ctx, hc.tx, "Company", cid).await?;
        if let Some(cc) = id(&co, "fa_depreciation_cost_center").or_else(|| id(&co, "default_cost_center")) {
            set(doc, "cost_center", cc.to_string());
        }
    }
    Ok(())
}

/// A draft's schedule and figures: the generated plan (or the manual one, ordered), as it
/// would be booked.
fn preview(doc: &mut Doc, precision: u32) {
    set(doc, "status", "Draft");
    let value = d(doc, "gross_purchase_amount") - d(doc, "opening_accumulated_depreciation");
    let manual = s(doc, "depreciation_method") == Some("Manual");
    if !b(doc, "calculate_depreciation") {
        put_rows(doc, vec![]);
    } else if manual {
        let list = schedule_rows(doc)
            .into_iter()
            .map(|mut r| {
                for f in ["journal_entry", "disposal", "posting_error"] {
                    r.insert(f.into(), Value::Null);
                }
                r.insert("booked".into(), json!(false));
                r
            })
            .collect();
        put_rows(doc, list);
    } else {
        let list = match plan_of(doc, precision) {
            Ok(plan) => plan.amounts(&plan.slots(), value).into_iter().map(|(on, a)| new_row(on, a)).collect(),
            Err(_) => vec![],
        };
        put_rows(doc, list);
    }
    columns(doc, Decimal::ZERO, Decimal::ZERO);
}

/// Moves a purchase booked to capital work in progress (or another category's account) to the
/// asset's fixed asset account, on its available-for-use date.
async fn capitalize(hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
    let (Some(pi), Some(line)) = (id(doc, "purchase_invoice"), id(doc, "purchase_invoice_item")) else { return Ok(()) };
    let inv = get(hc.kernel, hc.ctx, hc.tx, "PurchaseInvoice", pi).await?;
    if inv.get("docstatus").and_then(Value::as_i64) != Some(1) {
        return Err(Error::field("purchase_invoice", format!("purchase invoice {} is not submitted", named(&inv))));
    }
    let row =
        rows(&inv, "items").into_iter().find(|r| id(r, "id") == Some(line)).ok_or_else(|| Error::invalid("the purchase invoice line of this asset is gone"))?;
    let booked_to = id(&row, "expense_account").ok_or_else(|| Error::invalid("the purchase invoice line has no account"))?;
    let fixed_asset = id(doc, "fixed_asset_account").ok_or_else(|| Error::internal("asset without fixed asset account"))?;
    if booked_to == fixed_asset {
        return Ok(());
    }
    let asset = id(doc, "id").ok_or_else(|| Error::internal("asset without id"))?;
    let co = company(hc.kernel, hc.ctx, hc.tx, id(doc, "company").unwrap_or_default()).await?;
    let on = day(doc, "available_for_use_date").ok_or_else(|| Error::field("available_for_use_date", "is required"))?;
    let amount = d(doc, "gross_purchase_amount");
    let cc = id(doc, "cost_center");
    let mut entry = Entry::new(co.id, on, "Asset", asset, s(doc, "number").map(str::to_string));
    entry.memo = Some(format!("Capitalization of {} {}", named(doc), s(doc, "asset_name").unwrap_or_default()));
    entry.lines = vec![
        Line::new(fixed_asset, amount, &co.currency_code).cost_center(cc).remarks("Capitalized"),
        Line::new(booked_to, -amount, &co.currency_code).cost_center(cc).remarks("Capitalized"),
    ];
    ledger::post(hc.tx, hc.ctx, entry, co.precision).await?;
    Ok(())
}

pub struct AssetHook;

#[async_trait]
impl DocHook for AssetHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        // Only drafts are edited (a preview of a submitted asset must not re-plan it).
        if doc.get("docstatus").and_then(Value::as_i64).unwrap_or(0) != 0 {
            return Ok(());
        }
        defaults(hc, doc).await?;
        if let Some(cid) = id(doc, "company") {
            let co = company(hc.kernel, hc.ctx, hc.tx, cid).await?;
            preview(doc, co.precision);
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let mut errors = Vec::new();
        if let Some(item) = id(doc, "item") {
            let it = get(hc.kernel, hc.ctx, hc.tx, "Item", item).await?;
            if !b(&it, "fa_is_fixed_asset") {
                errors.push(FieldError::new("item", "is not a fixed asset item (Item → Is Fixed Asset)"));
            } else if b(&it, "is_stock_item") {
                errors.push(FieldError::new("item", "is a stock item"));
            }
        }
        let cid = id(doc, "company").ok_or_else(|| Error::field("company", "is required"))?;
        let co = company(hc.kernel, hc.ctx, hc.tx, cid).await?;
        for f in ["gross_purchase_amount", "opening_accumulated_depreciation", "salvage_value"] {
            if let Err(Error::Validation(e)) = check_places(doc, f, co.precision) {
                errors.extend(e);
            }
        }
        let gross = d(doc, "gross_purchase_amount");
        let opening = d(doc, "opening_accumulated_depreciation");
        if opening > gross {
            errors.push(FieldError::new("opening_accumulated_depreciation", "cannot exceed the gross purchase amount"));
        }
        if b(doc, "calculate_depreciation") {
            match plan_of(doc, co.precision) {
                Err(e) => errors.extend(e),
                Ok(plan) => {
                    let depreciable = gross - opening - plan.salvage;
                    if depreciable < Decimal::ZERO {
                        errors.push(FieldError::new("salvage_value", "cannot exceed the net book value"));
                    }
                    if plan.method == Method::Manual {
                        let list = schedule_rows(doc);
                        let mut previous: Option<NaiveDate> = None;
                        for (i, r) in list.iter().enumerate() {
                            let on = row_day(r);
                            if on < plan.available_for_use || previous.is_some_and(|p| on <= p) {
                                errors.push(
                                    FieldError::new("schedule_date", "dates must be on or after the available-for-use date and differ").in_row("schedule", i),
                                );
                            }
                            if d(r, "depreciation_amount") <= Decimal::ZERO || d(r, "depreciation_amount").normalize().scale() > co.precision {
                                errors.push(FieldError::new("depreciation_amount", "must be positive, in the currency's decimal places").in_row("schedule", i));
                            }
                            previous = Some(on);
                        }
                        let total: Decimal = list.iter().map(|r| d(r, "depreciation_amount")).sum();
                        if total > depreciable.max(Decimal::ZERO) {
                            errors.push(FieldError::new(
                                "schedule",
                                format!("the schedule depreciates {} but only {} is depreciable", total.normalize(), depreciable.normalize()),
                            ));
                        }
                    }
                }
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) }
    }

    async fn after_save(&self, _hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        // What the purchase invoice booked is what the asset carries.
        if let Some(old) = old.filter(|o| id(o, "purchase_invoice").is_some()) {
            for f in ["gross_purchase_amount", "asset_quantity", "company", "item", "purchase_date"] {
                if old.get(f) != doc.get(f) {
                    return Err(Error::field(f, "comes from the purchase invoice and cannot be changed"));
                }
            }
        }
        Ok(())
    }

    async fn before_submit(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if !b(doc, "is_existing_asset") && id(doc, "purchase_invoice").is_none() {
            return Err(Error::field(
                "purchase_invoice",
                "a new asset comes from its purchase invoice (Is Fixed Asset on the invoice line); mark assets already in the books as existing assets",
            ));
        }
        let category = id(doc, "asset_category").ok_or_else(|| Error::field("asset_category", "is required"))?;
        let cid = id(doc, "company").ok_or_else(|| Error::field("company", "is required"))?;
        let (_, accounts) = setup::accounts(hc.kernel, hc.ctx, hc.tx, category, cid).await?;
        set(doc, "fixed_asset_account", accounts.fixed_asset.to_string());
        if b(doc, "calculate_depreciation") {
            set(doc, "accumulated_depreciation_account", accounts.accumulated()?.to_string());
            set(doc, "depreciation_expense_account", accounts.expense()?.to_string());
        }
        Ok(())
    }

    async fn on_submit(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        capitalize(hc, doc).await?;
        let asset = id(doc, "id").ok_or_else(|| Error::internal("asset without id"))?;
        refresh(hc.kernel, hc.ctx, hc.tx, asset, None).await
    }

    async fn before_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if id(doc, "disposal").is_some() {
            return Err(Error::invalid("the asset was sold or scrapped; cancel the disposal first"));
        }
        if matches!(s(doc, "status"), Some("Out of Order" | "In Maintenance")) {
            return Err(Error::invalid("the asset has a repair or maintenance in progress; finish or delete it first"));
        }
        let asset = id(doc, "id").ok_or_else(|| Error::internal("asset without id"))?;
        let later: Option<String> = sqlx::query_scalar(
            "select number from e_asset_value_adjustment where tenant_id = $1 and asset = $2 and docstatus = 1 \
             union all select number from e_asset_repair where tenant_id = $1 and asset = $2 and docstatus = 1 and capitalize_repair_cost limit 1",
        )
        .bind(hc.ctx.tenant)
        .bind(asset)
        .fetch_optional(&mut **hc.tx)
        .await?;
        if let Some(n) = later {
            return Err(Error::invalid(format!("{n} changed the asset's value; cancel it first")));
        }
        Ok(())
    }

    async fn on_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let asset = id(doc, "id").ok_or_else(|| Error::internal("asset without id"))?;
        let (mut current, old) = open(hc.kernel, hc.ctx, hc.tx, asset).await?;
        while let Some(i) = schedule_rows(&current).iter().rposition(|r| b(r, "booked")) {
            unbook(hc.kernel, hc.ctx, hc.tx, &mut current, i).await?;
        }
        let co = company(hc.kernel, hc.ctx, hc.tx, id(doc, "company").unwrap_or_default()).await?;
        ledger::reverse(hc.tx, hc.ctx, "Asset", asset, hc.ctx.today(), co.precision).await?;
        save(hc.kernel, hc.ctx, hc.tx, current, &old, None).await?;
        Ok(())
    }

    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if !installed(hc.ctx) || doc.get("docstatus").and_then(Value::as_i64) != Some(0) {
            return Ok(());
        }
        if let Some(pi) = id(doc, "purchase_invoice") {
            let inv = get(hc.kernel, hc.ctx, hc.tx, "PurchaseInvoice", pi).await?;
            if inv.get("docstatus").and_then(Value::as_i64) == Some(1) {
                return Err(Error::invalid(format!(
                    "this asset carries part of purchase invoice {}; submit it, or cancel the invoice to remove its assets",
                    named(&inv)
                )));
            }
        }
        Ok(())
    }
}
