//! Changes to an asset's value: value adjustments (impairment or revaluation) and repairs, whose
//! cost is either capitalized (added to the asset and depreciated over the rest of its life) or
//! left where it was booked. Both first book the depreciation due by their date, post the change
//! and re-plan the rest of the schedule; cancelling reverses the posting and re-plans again.

use crate::asset::{self, day, in_service};
use crate::setup::check_account;
use crate::{check_places, named};
use async_trait::async_trait;
use ferrum_domain::ledger::{self, Entry, Line};
use ferrum_domain::util::{b, company, d, get, id, money, rows, s, set, sys};
use ferrum_kernel::{Doc, DocHook, Error, FieldError, HookCtx, Result};
use rust_decimal::Decimal;
use serde_json::Value;
use uuid::Uuid;

/// The asset a document refers to, read with the caller's own permissions (a user only acts on
/// assets they can see).
async fn readable_asset(hc: &mut HookCtx<'_>, doc: &Doc) -> Result<Doc> {
    let a = id(doc, "asset").ok_or_else(|| Error::field("asset", "is required"))?;
    hc.kernel.get_tx(hc.ctx, hc.tx, "Asset", a, false).await.map_err(|e| match e {
        Error::NotFound(_) => Error::field("asset", "not found"),
        other => other,
    })
}

/// Company, finance book and cost center follow the asset.
async fn follow_asset(hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<Option<Doc>> {
    let Some(a) = id(doc, "asset") else { return Ok(None) };
    let current = get(hc.kernel, hc.ctx, hc.tx, "Asset", a).await?;
    set(doc, "company", current.get("company").cloned().unwrap_or(Value::Null));
    if hc.entity.field("finance_book").is_some() {
        set(doc, "finance_book", current.get("finance_book").cloned().unwrap_or(Value::Null));
    }
    if id(doc, "cost_center").is_none() {
        set(doc, "cost_center", current.get("cost_center").cloned().unwrap_or(Value::Null));
    }
    Ok(Some(current))
}

/// Posts a value change of `amount` (positive = up) between the asset's fixed asset account and
/// `other`.
async fn post_change(hc: &mut HookCtx<'_>, doc: &Doc, current: &Doc, on: chrono::NaiveDate, amount: Decimal, other: Uuid, memo: String) -> Result<()> {
    let co = company(hc.kernel, hc.ctx, hc.tx, id(current, "company").unwrap_or_default()).await?;
    let fixed_asset = id(current, "fixed_asset_account").ok_or_else(|| Error::internal("asset without fixed asset account"))?;
    let me = id(doc, "id").ok_or_else(|| Error::internal("document without id"))?;
    let cc = id(doc, "cost_center").or_else(|| id(current, "cost_center"));
    let mut entry = Entry::new(co.id, on, &hc.entity.def.name, me, s(doc, "number").map(str::to_string));
    entry.memo = Some(memo.clone());
    entry.lines = vec![
        Line::new(fixed_asset, amount, &co.currency_code).cost_center(cc).remarks(memo.clone()),
        Line::new(other, -amount, &co.currency_code).cost_center(cc).remarks(memo),
    ];
    ledger::post(hc.tx, hc.ctx, entry, co.precision).await?;
    Ok(())
}

pub struct AdjustmentHook;

#[async_trait]
impl DocHook for AdjustmentHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if doc.get("docstatus").and_then(Value::as_i64).unwrap_or(0) != 0 {
            return Ok(());
        }
        if let Some(current) = follow_asset(hc, doc).await? {
            // A preview; the submit books the depreciation due by the date and fixes both figures.
            let value = d(&current, "value_after_depreciation");
            set(doc, "current_asset_value", money(value));
            set(doc, "difference_amount", money(d(doc, "new_asset_value") - value));
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let current = readable_asset(hc, doc).await?;
        in_service(&current)?;
        let on = day(doc, "adjustment_date").ok_or_else(|| Error::field("adjustment_date", "is required"))?;
        if on > hc.ctx.today() {
            return Err(Error::field("adjustment_date", "cannot be in the future"));
        }
        if day(&current, "available_for_use_date").is_some_and(|a| on < a) {
            return Err(Error::field("adjustment_date", "is before the asset was available for use"));
        }
        if let Some(until) = asset::booked_until(&current).filter(|u| on < *u) {
            return Err(Error::field("adjustment_date", format!("depreciation is booked up to {until}; date the adjustment on or after it")));
        }
        let later: Option<String> = sqlx::query_scalar(
            "select coalesce(number, id::text) from e_asset_value_adjustment where tenant_id = $1 and asset = $2 and docstatus = 1 and adjustment_date > $3 limit 1",
        )
        .bind(hc.ctx.tenant)
        .bind(id(doc, "asset"))
        .bind(on)
        .fetch_optional(&mut **hc.tx)
        .await?;
        if let Some(n) = later {
            return Err(Error::field("adjustment_date", format!("value adjustment {n} is dated later")));
        }
        let floor = if b(&current, "calculate_depreciation") { d(&current, "salvage_value") } else { Decimal::ZERO };
        if d(doc, "new_asset_value") < floor {
            return Err(Error::field("new_asset_value", format!("cannot be below the salvage value ({})", floor.normalize())));
        }
        let co = company(hc.kernel, hc.ctx, hc.tx, id(&current, "company").unwrap_or_default()).await?;
        check_places(doc, "new_asset_value", co.precision)?;
        if let Some(acc) = id(doc, "difference_account") {
            check_account(hc.kernel, hc.ctx, hc.tx, acc, co.id, "difference_account").await?;
        }
        Ok(())
    }

    /// The preview's difference must not count while the value it replaces is measured.
    async fn before_submit(&self, _hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        set(doc, "difference_amount", money(Decimal::ZERO));
        Ok(())
    }

    async fn on_submit(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let me = id(doc, "id").ok_or_else(|| Error::internal("adjustment without id"))?;
        let a = id(doc, "asset").ok_or_else(|| Error::field("asset", "is required"))?;
        let on = day(doc, "adjustment_date").ok_or_else(|| Error::field("adjustment_date", "is required"))?;
        let (mut current, old) = asset::open(hc.kernel, hc.ctx, hc.tx, a).await?;
        in_service(&current)?;
        let co = company(hc.kernel, hc.ctx, hc.tx, id(&current, "company").unwrap_or_default()).await?;
        asset::book_until(hc.kernel, hc.ctx, hc.tx, &mut current, on).await?;
        let value = asset::net_book_value(hc.tx, hc.ctx, &current).await?;
        let difference = d(doc, "new_asset_value") - value;
        if difference.is_zero() {
            return Err(Error::field("new_asset_value", "equals the asset's net book value on that date"));
        }
        let mut v = Doc::new();
        v.insert("current_asset_value".into(), money(value));
        v.insert("difference_amount".into(), money(difference));
        hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "AssetValueAdjustment", me, v).await?;
        let other = id(doc, "difference_account").ok_or_else(|| Error::field("difference_account", "is required"))?;
        let memo = format!("Value adjustment of {} from {} to {}", named(&current), value.normalize(), d(doc, "new_asset_value").normalize());
        post_change(hc, doc, &current, on, difference, other, memo).await?;
        asset::replan(hc.tx, hc.ctx, &mut current, difference, co.precision).await?;
        asset::save(hc.kernel, hc.ctx, hc.tx, current, &old, None).await?;
        Ok(())
    }

    async fn before_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let current = get(hc.kernel, hc.ctx, hc.tx, "Asset", id(doc, "asset").unwrap_or_default()).await?;
        in_service(&current)?;
        let later: Option<String> = sqlx::query_scalar(
            "select coalesce(number, id::text) from e_asset_value_adjustment where tenant_id = $1 and asset = $2 and docstatus = 1 and id <> $3 \
             and (adjustment_date, created_at) > ($4::date, (select created_at from e_asset_value_adjustment where tenant_id = $1 and id = $3)) limit 1",
        )
        .bind(hc.ctx.tenant)
        .bind(id(doc, "asset"))
        .bind(id(doc, "id"))
        .bind(day(doc, "adjustment_date"))
        .fetch_optional(&mut **hc.tx)
        .await?;
        if let Some(n) = later {
            return Err(Error::invalid(format!("value adjustment {n} came after this one; cancel it first")));
        }
        Ok(())
    }

    async fn on_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let me = id(doc, "id").ok_or_else(|| Error::internal("adjustment without id"))?;
        let a = id(doc, "asset").ok_or_else(|| Error::field("asset", "is required"))?;
        let (mut current, old) = asset::open(hc.kernel, hc.ctx, hc.tx, a).await?;
        let co = company(hc.kernel, hc.ctx, hc.tx, id(&current, "company").unwrap_or_default()).await?;
        ledger::reverse(hc.tx, hc.ctx, "AssetValueAdjustment", me, hc.ctx.today(), co.precision).await?;
        asset::replan(hc.tx, hc.ctx, &mut current, -d(doc, "difference_amount"), co.precision).await?;
        asset::save(hc.kernel, hc.ctx, hc.tx, current, &old, None).await?;
        Ok(())
    }
}

pub struct RepairHook;

fn capitalized(doc: &Doc) -> bool {
    s(doc, "repair_status") == Some("Completed") && b(doc, "capitalize_repair_cost") && d(doc, "repair_cost") > Decimal::ZERO
}

#[async_trait]
impl DocHook for RepairHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if doc.get("docstatus").and_then(Value::as_i64).unwrap_or(0) != 0 {
            return Ok(());
        }
        follow_asset(hc, doc).await?;
        if b(doc, "capitalize_repair_cost")
            && id(doc, "expense_account").is_none()
            && let Some(pi) = id(doc, "purchase_invoice")
        {
            let inv = get(hc.kernel, hc.ctx, hc.tx, "PurchaseInvoice", pi).await?;
            if let Some(acc) = rows(&inv, "items").iter().find_map(|r| id(r, "expense_account")) {
                set(doc, "expense_account", acc.to_string());
            }
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let current = readable_asset(hc, doc).await?;
        in_service(&current)?;
        let today = hc.ctx.today();
        let mut errors = Vec::new();
        if day(doc, "failure_date").is_some_and(|f| f > today) || day(doc, "completion_date").is_some_and(|c| c > today) {
            errors.push(FieldError::new("completion_date", "repair dates cannot be in the future"));
        }
        if day(doc, "failure_date").zip(day(&current, "purchase_date")).is_some_and(|(f, p)| f < p) {
            errors.push(FieldError::new("failure_date", "is before the asset was bought"));
        }
        if !errors.is_empty() {
            return Err(Error::Validation(errors));
        }
        let co = company(hc.kernel, hc.ctx, hc.tx, id(&current, "company").unwrap_or_default()).await?;
        check_places(doc, "repair_cost", co.precision)?;
        if b(doc, "capitalize_repair_cost")
            && let Some(acc) = id(doc, "expense_account")
        {
            check_account(hc.kernel, hc.ctx, hc.tx, acc, co.id, "expense_account").await?;
        }
        if let Some(pi) = id(doc, "purchase_invoice") {
            let inv = get(hc.kernel, hc.ctx, hc.tx, "PurchaseInvoice", pi).await?;
            if id(&inv, "company") != Some(co.id) || inv.get("docstatus").and_then(Value::as_i64) != Some(1) {
                return Err(Error::field("purchase_invoice", "must be a submitted invoice of the asset's company"));
            }
        }
        Ok(())
    }

    /// A pending repair puts the asset out of order (and a changed one may take it back).
    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        for a in [id(doc, "asset"), old.and_then(|o| id(o, "asset"))].into_iter().flatten().collect::<std::collections::BTreeSet<_>>() {
            asset::refresh(hc.kernel, hc.ctx, hc.tx, a, None).await?;
        }
        Ok(())
    }

    async fn before_submit(&self, _hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if s(doc, "repair_status") == Some("Pending") {
            return Err(Error::field("repair_status", "complete or cancel the repair before submitting it"));
        }
        Ok(())
    }

    async fn on_submit(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let a = id(doc, "asset").ok_or_else(|| Error::field("asset", "is required"))?;
        if !capitalized(doc) {
            return asset::refresh(hc.kernel, hc.ctx, hc.tx, a, None).await;
        }
        let on = day(doc, "completion_date").ok_or_else(|| Error::field("completion_date", "is required"))?;
        let (mut current, old) = asset::open(hc.kernel, hc.ctx, hc.tx, a).await?;
        in_service(&current)?;
        let co = company(hc.kernel, hc.ctx, hc.tx, id(&current, "company").unwrap_or_default()).await?;
        asset::book_until(hc.kernel, hc.ctx, hc.tx, &mut current, on).await?;
        let cost = d(doc, "repair_cost");
        let other = id(doc, "expense_account").ok_or_else(|| Error::field("expense_account", "is required"))?;
        post_change(hc, doc, &current, on, cost, other, format!("Capitalized repair of {}", named(&current))).await?;
        asset::replan(hc.tx, hc.ctx, &mut current, cost, co.precision).await?;
        asset::save(hc.kernel, hc.ctx, hc.tx, current, &old, None).await?;
        Ok(())
    }

    async fn before_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if capitalized(doc) {
            let current = get(hc.kernel, hc.ctx, hc.tx, "Asset", id(doc, "asset").unwrap_or_default()).await?;
            in_service(&current)?;
        }
        Ok(())
    }

    async fn on_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let a = id(doc, "asset").ok_or_else(|| Error::field("asset", "is required"))?;
        if !capitalized(doc) {
            return asset::refresh(hc.kernel, hc.ctx, hc.tx, a, None).await;
        }
        let me = id(doc, "id").ok_or_else(|| Error::internal("repair without id"))?;
        let (mut current, old) = asset::open(hc.kernel, hc.ctx, hc.tx, a).await?;
        let co = company(hc.kernel, hc.ctx, hc.tx, id(&current, "company").unwrap_or_default()).await?;
        ledger::reverse(hc.tx, hc.ctx, "AssetRepair", me, hc.ctx.today(), co.precision).await?;
        asset::replan(hc.tx, hc.ctx, &mut current, -d(doc, "repair_cost"), co.precision).await?;
        asset::save(hc.kernel, hc.ctx, hc.tx, current, &old, None).await?;
        Ok(())
    }

    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let (Some(a), Some(me)) = (id(doc, "asset"), id(doc, "id")) {
            asset::refresh(hc.kernel, hc.ctx, hc.tx, a, Some(me)).await?;
        }
        Ok(())
    }
}
