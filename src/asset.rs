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
use ferrum_app_erp::ledger::{self, Entry, Line};
use ferrum_domain::util::{b, company, d, elevated, get, id, money, rows, s, set, sys};
use ferrum_kernel::value::parse_date;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, FieldError, HookCtx, Kernel, Result, Tx, store};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::collections::HashMap;
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

/// Which depreciation something is about: the asset's own, which every finance book has (`None`),
/// or the depreciation a finance book keeps instead.
pub(crate) type Book = Option<Uuid>;

/// The finance books that keep their own depreciation of the asset.
pub(crate) fn books(doc: &Doc) -> Vec<Uuid> {
    rows(doc, "finance_books").iter().filter_map(|r| id(r, "finance_book")).collect()
}

/// The asset's own depreciation, then each finance book's.
pub(crate) fn every(doc: &Doc) -> Vec<Book> {
    std::iter::once(None).chain(books(doc).into_iter().map(Some)).collect()
}

/// What a finance book may set for itself; what it leaves empty is the asset's.
const BOOK_FIELDS: &[&str] = &[
    "depreciation_method",
    "total_number_of_depreciations",
    "frequency_of_depreciation",
    "rate_of_depreciation",
    "salvage_value",
    "opening_accumulated_depreciation",
    "opening_number_of_booked_depreciations",
    "depreciation_start_date",
];

/// The asset as `book` sees it: the book's settings in place of the asset's.
pub(crate) fn as_in(doc: &Doc, book: Book) -> Doc {
    let mut v = doc.clone();
    if let Some(row) = book.and_then(|bk| rows(doc, "finance_books").into_iter().find(|r| id(r, "finance_book") == Some(bk))) {
        for f in BOOK_FIELDS {
            if let Some(x) = row.get(*f).filter(|x| !x.is_null()) {
                v.insert(f.to_string(), x.clone());
            }
        }
    }
    v
}

fn table(book: Book) -> &'static str {
    if book.is_some() { "book_schedule" } else { "schedule" }
}

/// The schedule `book` depreciates by, by date (booked rows first: they are always the oldest).
fn rows_in(doc: &Doc, book: Book) -> Vec<Doc> {
    let mut list: Vec<Doc> = rows(doc, table(book)).into_iter().filter(|r| book.is_none() || id(r, "finance_book") == book).collect();
    list.sort_by_key(|r| (s(r, "schedule_date").unwrap_or_default().to_string(), !b(r, "booked")));
    list
}

fn put_in(doc: &mut Doc, book: Book, list: Vec<Doc>) {
    let mut all: Vec<Doc> = match book {
        None => vec![],
        Some(_) => rows(doc, "book_schedule").into_iter().filter(|r| id(r, "finance_book") != book).collect(),
    };
    all.extend(list.into_iter().map(|mut r| {
        if let Some(bk) = book {
            set(&mut r, "finance_book", bk.to_string());
        }
        r
    }));
    set(doc, table(book), Value::Array(all.into_iter().map(Value::Object).collect()));
}

fn schedule_rows(doc: &Doc) -> Vec<Doc> {
    rows_in(doc, None)
}

fn put_rows(doc: &mut Doc, list: Vec<Doc>) {
    put_in(doc, None, list)
}

/// The asset's depreciation plan as `book` sees it.
pub(crate) fn plan_for(doc: &Doc, book: Book, precision: u32) -> std::result::Result<Plan, Vec<FieldError>> {
    plan_of(&as_in(doc, book), precision)
}

fn new_row(on: NaiveDate, amount: Decimal) -> Doc {
    obj(json!({"id": Uuid::now_v7().to_string(), "schedule_date": on.to_string(), "depreciation_amount": money(amount), "booked": false}))
}

fn row_day(r: &Doc) -> NaiveDate {
    day(r, "schedule_date").unwrap_or(NaiveDate::MIN)
}

/// Capitalized repairs and the value adjustments every book has, and the adjustments each
/// finance book made alone.
async fn changes(tx: &mut Tx, tenant: Uuid, asset: Uuid) -> Result<(Decimal, Decimal, HashMap<Uuid, Decimal>)> {
    let (repairs, adjustments): (Decimal, Decimal) = sqlx::query_as(
        "select coalesce((select sum(repair_cost + coalesce(parts_cost, 0)) from e_asset_repair where tenant_id = $1 and asset = $2 and docstatus = 1 and capitalize_repair_cost), 0), \
         coalesce((select sum(difference_amount) from e_asset_value_adjustment where tenant_id = $1 and asset = $2 and docstatus = 1 and finance_book is null), 0)",
    )
    .bind(tenant)
    .bind(asset)
    .fetch_one(&mut **tx)
    .await?;
    let books: Vec<(Uuid, Decimal)> = sqlx::query_as(
        "select finance_book, sum(difference_amount) from e_asset_value_adjustment \
         where tenant_id = $1 and asset = $2 and docstatus = 1 and finance_book is not null group by finance_book",
    )
    .bind(tenant)
    .bind(asset)
    .fetch_all(&mut **tx)
    .await?;
    Ok((repairs, adjustments, books.into_iter().collect()))
}

/// Fills each schedule's running total and order, and the asset's figures from them: its own on
/// the asset, each finance book's on its row of finance books.
fn columns(doc: &mut Doc, repairs: Decimal, adjustments: Decimal, by_book: &HashMap<Uuid, Decimal>) {
    let cost = d(doc, "gross_purchase_amount") + repairs + adjustments;
    let mut figures: HashMap<Book, (Decimal, Decimal, Value)> = HashMap::new();
    for book in every(doc) {
        let opening = d(&as_in(doc, book), "opening_accumulated_depreciation");
        let mut list = rows_in(doc, book);
        let (mut running, mut booked) = (opening, opening);
        let mut next = Value::Null;
        for r in list.iter_mut() {
            let a = d(r, "depreciation_amount");
            running += a;
            set(r, "accumulated_depreciation", money(running));
            if b(r, "booked") {
                booked += a;
            } else if next.is_null() {
                next = r.get("schedule_date").cloned().unwrap_or(Value::Null);
            }
        }
        put_in(doc, book, list);
        let cost = cost + book.and_then(|bk| by_book.get(&bk)).copied().unwrap_or_default();
        figures.insert(book, (cost, booked, next));
    }
    for t in ["schedule", "book_schedule"] {
        let mut list = rows(doc, t);
        for (i, r) in list.iter_mut().enumerate() {
            set(r, "idx", i);
        }
        set(doc, t, Value::Array(list.into_iter().map(Value::Object).collect()));
    }
    let mut kept = rows(doc, "finance_books");
    for r in kept.iter_mut() {
        if let Some((cost, booked, next)) = figures.get(&id(r, "finance_book")).filter(|_| id(r, "finance_book").is_some()) {
            set(r, "accumulated_depreciation", money(*booked));
            set(r, "value_after_depreciation", money(*cost - *booked));
            set(r, "next_depreciation_date", next.clone());
        }
    }
    set(doc, "finance_books", Value::Array(kept.into_iter().map(Value::Object).collect()));
    let (_, booked, next) = figures.remove(&None).unwrap_or((cost, Decimal::ZERO, Value::Null));
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
    let (repairs, adjustments, by_book) = changes(tx, ctx.tenant, asset).await?;
    columns(&mut doc, repairs, adjustments, &by_book);
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

/// Net book value now in `book`: cost (with capitalized repairs and value adjustments, the book's
/// own among them) less opening and booked depreciation, counting rows booked in memory.
pub(crate) async fn net_book_value(tx: &mut Tx, ctx: &Ctx, doc: &Doc, book: Book) -> Result<Decimal> {
    let asset = id(doc, "id").ok_or_else(|| Error::internal("asset without id"))?;
    let (repairs, adjustments, by_book) = changes(tx, ctx.tenant, asset).await?;
    let own = book.and_then(|bk| by_book.get(&bk)).copied().unwrap_or_default();
    Ok(d(doc, "gross_purchase_amount") + repairs + adjustments + own - accumulated(doc, book))
}

/// Opening and booked depreciation in `book`, counting rows booked in memory.
pub(crate) fn accumulated(doc: &Doc, book: Book) -> Decimal {
    d(&as_in(doc, book), "opening_accumulated_depreciation")
        + rows_in(doc, book).iter().filter(|r| b(r, "booked")).map(|r| d(r, "depreciation_amount")).sum::<Decimal>()
}

/// The latest booked depreciation date, in any book.
pub(crate) fn booked_until(doc: &Doc) -> Option<NaiveDate> {
    every(doc).into_iter().flat_map(|bk| rows_in(doc, bk)).filter(|r| b(r, "booked")).map(|r| row_day(&r)).max()
}

/// The fields `carry_dates` keeps.
pub(crate) const DATES: &[&str] = &["expected_return_date", "last_verified_date", "verification_result", "next_verification_date"];

/// What an asset carries of its movements and verifications, worked out from those that stand
/// (never accumulated): when it is due back, and the last verification's day, finding and next due
/// date. An asset out of service (disposed of, cancelled) is due nothing, so no list of what is
/// due shows it; the day it was last verified stays.
pub(crate) async fn carry_dates(tx: &mut Tx, tenant: Uuid, doc: &mut Doc, in_service: bool) -> Result<()> {
    let asset = id(doc, "id").ok_or_else(|| Error::internal("asset without id"))?;
    let latest = crate::verification::standing(tx, tenant, asset).await?;
    let due = if in_service { crate::movement::due_back(tx, tenant, asset).await? } else { None };
    set(doc, "last_verified_date", json!(latest.as_ref().map(|l| l.0.to_string())));
    let live = latest.filter(|_| in_service);
    set(doc, "verification_result", json!(live.as_ref().map(|l| l.1.clone())));
    set(doc, "next_verification_date", json!(live.and_then(|l| l.2).map(|n| n.to_string())));
    set(doc, "expected_return_date", json!(due.map(|d| d.to_string())));
    Ok(())
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

/// Posts depreciation of `amount` on `on` as a submitted journal entry in `book` (`None`: every
/// book): debit depreciation expense, credit accumulated depreciation, on the asset's cost center;
/// `back` the other way round.
async fn post_depreciation(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &Doc, on: NaiveDate, amount: Decimal, book: Book, back: bool) -> Result<Uuid> {
    let expense = id(doc, "depreciation_expense_account").ok_or_else(|| Error::invalid("the asset has no depreciation expense account"))?;
    let accumulated = id(doc, "accumulated_depreciation_account").ok_or_else(|| Error::invalid("the asset has no accumulated depreciation account"))?;
    let cc = id(doc, "cost_center").map(|c| c.to_string());
    let what = match (book, back) {
        (Some(_), true) => {
            format!("Depreciation of {} {} in the common books, which this book keeps its own of", named(doc), s(doc, "asset_name").unwrap_or_default())
        }
        _ => format!("Depreciation of {} {}", named(doc), s(doc, "asset_name").unwrap_or_default()),
    };
    let (dr, cr) = if back { (accumulated, expense) } else { (expense, accumulated) };
    let input = json!({
        "company": doc.get("company"), "posting_date": on.to_string(), "entry_type": "Depreciation", "fa_asset": doc.get("id"), "remark": what,
        "finance_book": book.map(|bk| bk.to_string()),
        "accounts": [
            {"account": dr.to_string(), "debit": money(amount), "cost_center": cc, "remarks": what},
            {"account": cr.to_string(), "credit": money(amount), "cost_center": cc, "remarks": what},
        ],
    });
    let engine = elevated(ctx);
    let je = k.insert_tx(&engine, tx, "JournalEntry", obj(input)).await?;
    let je = id(&je, "id").ok_or_else(|| Error::internal("journal entry without id"))?;
    k.submit_tx(&engine, tx, "JournalEntry", je, None).await?;
    Ok(je)
}

/// Books row `i` of `book`'s schedule on its date. A row of the asset's own schedule is in every
/// book, so each finance book that keeps its own depreciation takes it out again.
async fn book(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &mut Doc, book: Book, i: usize) -> Result<()> {
    let mut list = rows_in(doc, book);
    let amount = d(&list[i], "depreciation_amount");
    let on = row_day(&list[i]);
    let je = post_depreciation(k, ctx, tx, doc, on, amount, book, false).await?;
    if book.is_none() {
        let mut out = Vec::new();
        for bk in books(doc) {
            let back = post_depreciation(k, ctx, tx, doc, on, amount, Some(bk), true).await?;
            out.push(json!({"finance_book": bk.to_string(), "journal_entry": back.to_string()}));
        }
        set(&mut list[i], "book_entries", if out.is_empty() { Value::Null } else { Value::Array(out) });
    }
    set(&mut list[i], "journal_entry", je.to_string());
    set(&mut list[i], "booked", true);
    set(&mut list[i], "posting_error", Value::Null);
    put_in(doc, book, list);
    Ok(())
}

/// Unbooks row `i` of `book`'s schedule: the row lets go of its journal entries first (the journal
/// entry guard refuses to cancel an entry a schedule row still books), then they are cancelled
/// (reversed).
async fn unbook(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &mut Doc, book: Book, i: usize) -> Result<()> {
    let mut list = rows_in(doc, book);
    if let Some(row) = id(&list[i], "id") {
        let mut v = Doc::new();
        v.insert("journal_entry".into(), Value::Null);
        v.insert("booked".into(), json!(false));
        if book.is_none() {
            v.insert("book_entries".into(), Value::Null);
        }
        match k.set_child_values_tx(&sys(ctx), tx, "Asset", table(book), row, v).await {
            Ok(()) | Err(Error::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    let taken_out = list[i].get("book_entries").and_then(Value::as_array).cloned().unwrap_or_default();
    for je in id(&list[i], "journal_entry").into_iter().chain(taken_out.iter().filter_map(|e| e.get("journal_entry")?.as_str()?.parse().ok())) {
        k.cancel_tx(&elevated(ctx), tx, "JournalEntry", je, None).await?;
    }
    set(&mut list[i], "journal_entry", Value::Null);
    set(&mut list[i], "booked", false);
    set(&mut list[i], "book_entries", Value::Null);
    put_in(doc, book, list);
    Ok(())
}

/// Books the earliest unbooked row of any book if it is due by `until` (the asset's own first on
/// the same day); `false` when there is none.
pub(crate) async fn book_next(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &mut Doc, until: NaiveDate) -> Result<bool> {
    let mut next: Option<(NaiveDate, usize, Book, usize)> = None;
    for (order, bk) in every(doc).into_iter().enumerate() {
        let list = rows_in(doc, bk);
        if let Some(i) = list.iter().position(|r| !b(r, "booked"))
            && row_day(&list[i]) <= until
            && next.as_ref().is_none_or(|n| (row_day(&list[i]), order) < (n.0, n.1))
        {
            next = Some((row_day(&list[i]), order, bk, i));
        }
    }
    match next {
        Some((_, _, bk, i)) => {
            book(k, ctx, tx, doc, bk, i).await?;
            Ok(true)
        }
        None => Ok(false),
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

/// Re-plans every book's unbooked rows after the asset's value changed by `change` in all of them.
pub(crate) async fn replan(tx: &mut Tx, ctx: &Ctx, doc: &mut Doc, change: Decimal, precision: u32) -> Result<()> {
    for bk in every(doc) {
        replan_in(tx, ctx, doc, bk, change, precision).await?;
    }
    Ok(())
}

/// Re-plans `book`'s unbooked rows after the asset's value in it changed by `change` (the document
/// making the change is already counted in the asset's figures). Generated methods recompute the
/// periods left from the net book value; a manual plan is rescaled in proportion to the value it
/// depreciates. Never below salvage.
pub(crate) async fn replan_in(tx: &mut Tx, ctx: &Ctx, doc: &mut Doc, book: Book, change: Decimal, precision: u32) -> Result<()> {
    if !b(doc, "calculate_depreciation") {
        return Ok(());
    }
    let plan = plan_for(doc, book, precision).map_err(Error::Validation)?;
    let value = net_book_value(tx, ctx, doc, book).await?;
    if value < plan.salvage {
        return Err(Error::invalid(format!(
            "this would take asset {}'s net book value ({}) below its salvage value ({})",
            named(doc),
            value.normalize(),
            plan.salvage.normalize()
        )));
    }
    let (booked, planned): (Vec<Doc>, Vec<Doc>) = rows_in(doc, book).into_iter().partition(|r| b(r, "booked"));
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
    put_in(doc, book, list);
    Ok(())
}

/// Adds `months` (fewer when negative) to the useful life of the asset and of each finance book
/// that keeps its own depreciation, in whole periods of each; the caller re-plans.
pub(crate) fn extend_life(doc: &mut Doc, months: i64) -> Result<()> {
    if months == 0 {
        return Ok(());
    }
    if !b(doc, "calculate_depreciation") || s(doc, "depreciation_method") == Some("Manual") {
        return Err(Error::field("increase_in_asset_life", "only a generated depreciation plan is extended (a manual schedule is changed by hand)"));
    }
    let mut lives = Vec::new();
    for bk in every(doc) {
        let seen = as_in(doc, bk);
        let frequency = int(&seen, "frequency_of_depreciation").max(1);
        let whose = if bk.is_some() { "a finance book's" } else { "the asset's" };
        if months % frequency != 0 {
            return Err(Error::field("increase_in_asset_life", format!("{} months is not a whole number of {whose} {frequency}-month periods", months.abs())));
        }
        let periods = int(&seen, "total_number_of_depreciations") + months / frequency;
        if periods < 1 {
            return Err(Error::field("increase_in_asset_life", format!("would leave {whose} life without a period")));
        }
        lives.push((bk, periods));
    }
    for (bk, periods) in lives {
        match bk {
            None => set(doc, "total_number_of_depreciations", periods),
            Some(_) => {
                let mut list = rows(doc, "finance_books");
                for r in list.iter_mut().filter(|r| id(r, "finance_book") == bk) {
                    set(r, "total_number_of_depreciations", periods);
                }
                set(doc, "finance_books", Value::Array(list.into_iter().map(Value::Object).collect()));
            }
        }
    }
    Ok(())
}

/// Books depreciation up to a disposal date in every book: rows booked after it are unbooked, rows
/// due by it are booked, the running period is depreciated for its days up to the date (a row of
/// its own, marked with the disposal) and the plan after it is removed. Returns the removed rows,
/// which a cancelled disposal puts back: `{"schedule": [...], "books": [{"finance_book", "rows"}]}`.
pub(crate) async fn depreciate_to_disposal(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &mut Doc, on: NaiveDate, disposal: Uuid, precision: u32) -> Result<Value> {
    for bk in every(doc) {
        while let Some(i) = rows_in(doc, bk).iter().rposition(|r| b(r, "booked") && row_day(r) > on) {
            unbook(k, ctx, tx, doc, bk, i).await?;
        }
    }
    book_until(k, ctx, tx, doc, on).await?;
    let mut snapshot = json!({"schedule": [], "books": []});
    for bk in every(doc) {
        let removed = cut_at(k, ctx, tx, doc, bk, on, disposal, precision).await?;
        match bk {
            None => snapshot["schedule"] = removed,
            Some(b) => {
                if let Some(list) = snapshot["books"].as_array_mut() {
                    list.push(json!({"finance_book": b.to_string(), "rows": removed}));
                }
            }
        }
    }
    Ok(snapshot)
}

/// `book`'s running period depreciated up to the disposal date and the rest of its plan removed.
#[allow(clippy::too_many_arguments)]
async fn cut_at(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &mut Doc, book: Book, on: NaiveDate, disposal: Uuid, precision: u32) -> Result<Value> {
    let mut list = rows_in(doc, book);
    let Some(i) = list.iter().position(|r| !b(r, "booked")) else { return Ok(json!([])) };
    let removed = list.split_off(i);
    let snapshot: Vec<Value> =
        removed.iter().map(|r| json!({"schedule_date": r.get("schedule_date"), "depreciation_amount": r.get("depreciation_amount")})).collect();
    let plan = plan_for(doc, book, precision).map_err(Error::Validation)?;
    let next = &removed[0];
    let start = plan.period_start(row_day(next), list.last().map(row_day));
    let amount = schedule::partial(d(next, "depreciation_amount"), start, row_day(next), on, precision);
    if amount > Decimal::ZERO {
        let mut row = new_row(on, amount);
        set(&mut row, "disposal", disposal.to_string());
        list.push(row);
    }
    put_in(doc, book, list);
    if let Some(i) = rows_in(doc, book).iter().position(|r| id(r, "disposal") == Some(disposal)) {
        self::book(k, ctx, tx, doc, book, i).await?;
    }
    Ok(Value::Array(snapshot))
}

/// Undoes `depreciate_to_disposal`: each book's disposal row is unbooked and removed, and the plan
/// it removed comes back (rows it had to unbook come back unbooked, to be booked again). A
/// snapshot from before finance books kept their own depreciation is the asset's rows alone.
pub(crate) async fn restore_after_disposal(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &mut Doc, disposal: Uuid, snapshot: &Value) -> Result<()> {
    let mut removed: Vec<(Book, Value)> = match snapshot {
        Value::Array(_) => vec![(None, snapshot.clone())],
        _ => vec![(None, snapshot["schedule"].clone())],
    };
    for b in snapshot["books"].as_array().into_iter().flatten() {
        if let Some(bk) = b["finance_book"].as_str().and_then(|v| v.parse().ok()) {
            removed.push((Some(bk), b["rows"].clone()));
        }
    }
    for (bk, rows_removed) in removed {
        if let Some(i) = rows_in(doc, bk).iter().position(|r| id(r, "disposal") == Some(disposal)) {
            unbook(k, ctx, tx, doc, bk, i).await?;
            let mut list = rows_in(doc, bk);
            list.remove(i);
            put_in(doc, bk, list);
        }
        let mut list = rows_in(doc, bk);
        for r in rows_removed.as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(Value::as_object) {
            if let Some(on) = day(r, "schedule_date") {
                list.push(new_row(on, d(r, "depreciation_amount")));
            }
        }
        put_in(doc, bk, list);
    }
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
    // Each finance book's first depreciation date, by its own frequency.
    if b(doc, "calculate_depreciation")
        && let Some(afu) = day(doc, "available_for_use_date")
    {
        let mut list = rows(doc, "finance_books");
        for r in list.iter_mut() {
            let seen = as_in(doc, id(r, "finance_book"));
            let frequency = int(&seen, "frequency_of_depreciation");
            if frequency > 0 && day(r, "depreciation_start_date").is_none_or(|start| start < afu) {
                let behind = int(&seen, "opening_number_of_booked_depreciations").clamp(0, 1200);
                let first = schedule::add_months(schedule::default_start(afu, frequency as u32), (behind * frequency) as i32);
                set(r, "depreciation_start_date", first.to_string());
            }
        }
        set(doc, "finance_books", Value::Array(list.into_iter().map(Value::Object).collect()));
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
    set(doc, "book_schedule", Value::Array(vec![]));
    if b(doc, "calculate_depreciation") {
        for bk in books(doc) {
            let seen = as_in(doc, Some(bk));
            let value = d(&seen, "gross_purchase_amount") - d(&seen, "opening_accumulated_depreciation");
            if let Ok(plan) = plan_of(&seen, precision) {
                put_in(doc, Some(bk), plan.amounts(&plan.slots(), value).into_iter().map(|(on, a)| new_row(on, a)).collect());
            }
        }
    }
    columns(doc, Decimal::ZERO, Decimal::ZERO, &HashMap::new());
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

/// What Edit Details changes: text facts, then dates.
const DETAIL_TEXTS: &[&str] = &["serial_number", "manufacturer", "model_number"];
const DETAIL_DATES: &[&str] = &["warranty_expiry_date", "end_of_life_date"];

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
        // A finance book keeps its own depreciation once, by a generated plan.
        let mut seen = std::collections::BTreeSet::new();
        for (i, r) in rows(doc, "finance_books").iter().enumerate() {
            let Some(bk) = id(r, "finance_book") else { continue };
            if !seen.insert(bk) {
                errors.push(FieldError::new("finance_book", "is listed twice").in_row("finance_books", i));
            }
            if !b(doc, "calculate_depreciation") {
                errors.push(FieldError::new("finance_book", "the asset does not depreciate").in_row("finance_books", i));
                continue;
            }
            match plan_for(doc, Some(bk), co.precision) {
                Err(e) => errors.extend(e.into_iter().map(|f| f.in_row("finance_books", i))),
                Ok(plan) if plan.method == Method::Manual => {
                    errors.push(FieldError::new("depreciation_method", "a finance book needs a generated method").in_row("finance_books", i));
                }
                Ok(plan) => {
                    let seen_in = as_in(doc, Some(bk));
                    if d(&seen_in, "opening_accumulated_depreciation") + plan.salvage > gross {
                        errors.push(FieldError::new("salvage_value", "cannot exceed the net book value").in_row("finance_books", i));
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
        for bk in every(&current) {
            while let Some(i) = rows_in(&current, bk).iter().rposition(|r| b(r, "booked")) {
                unbook(hc.kernel, hc.ctx, hc.tx, &mut current, bk, i).await?;
            }
        }
        let co = company(hc.kernel, hc.ctx, hc.tx, id(doc, "company").unwrap_or_default()).await?;
        ledger::reverse(hc.tx, hc.ctx, "Asset", asset, hc.ctx.today(), co.precision).await?;
        carry_dates(hc.tx, hc.ctx.tenant, &mut current, false).await?;
        save(hc.kernel, hc.ctx, hc.tx, current, &old, None).await?;
        Ok(())
    }

    /// Edit Details: serial number, maker, model, warranty and end of life are facts about the
    /// thing, not part of what was booked, so they stay editable on a submitted asset (which the
    /// form cannot do). A field left out of the parameters stays as it is; an empty one is cleared.
    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if name != "update_details" {
            return Ok(None);
        }
        let asset = id(doc, "id").ok_or_else(|| Error::internal("asset without id"))?;
        let mut v = Doc::new();
        let mut errors = Vec::new();
        for f in DETAIL_TEXTS.iter().chain(DETAIL_DATES) {
            let text = match params.get(*f) {
                None => continue,
                Some(Value::Null) => "",
                Some(Value::String(t)) => t.trim(),
                Some(_) => {
                    errors.push(FieldError::new(*f, "must be text"));
                    continue;
                }
            };
            if text.is_empty() {
                v.insert(f.to_string(), Value::Null);
            } else if DETAIL_DATES.contains(f) {
                match parse_date(text) {
                    Ok(on) => v.insert(f.to_string(), json!(on.to_string())),
                    Err(_) => {
                        errors.push(FieldError::new(*f, "is not a date (use YYYY-MM-DD)"));
                        continue;
                    }
                };
            } else if text.chars().count() > 120 {
                errors.push(FieldError::new(*f, "is longer than 120 characters"));
            } else {
                v.insert(f.to_string(), json!(text));
            }
        }
        if !errors.is_empty() {
            return Err(Error::Validation(errors));
        }
        if !v.is_empty() {
            hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "Asset", asset, v).await?;
        }
        Ok(Some(json!({"message": "Details saved"})))
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
