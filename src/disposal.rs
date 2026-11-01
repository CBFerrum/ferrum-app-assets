//! Disposal. Scrapping writes the asset off when the disposal is submitted. Selling goes through
//! a sales invoice made by the disposal's action: its line is credited to the asset's fixed asset
//! account, and submitting the invoice submits the disposal, which books the depreciation due by
//! the sale date and the gain or loss. Cancelling the invoice cancels the disposal, which reverses
//! its entries and puts the asset's schedule back.
//!
//! The disposal entry closes the asset out of the books: debit accumulated depreciation (opening
//! included), credit the fixed asset account with the cost not already credited by the invoice,
//! and the difference — proceeds less net book value — to the gain/loss account.

use crate::asset::{self, day, in_service};
use crate::setup;
use crate::{check_places, installed, named};
use async_trait::async_trait;
use ferrum_app_erp::ledger::{self, Entry, Line};
use ferrum_domain::flow::create_for_user;
use ferrum_domain::flow::created;
use ferrum_domain::util::{b, company, d, elevated, get, id, money, rows, s, set, sys};
use ferrum_kernel::{Doc, DocHook, Error, FieldError, HookCtx, Result};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::collections::HashSet;
use uuid::Uuid;

fn docstatus(doc: &Doc) -> i64 {
    doc.get("docstatus").and_then(Value::as_i64).unwrap_or(0)
}

/// Sales invoice lines that sell an asset, with their disposal.
fn asset_lines(si: &Doc) -> Vec<(Uuid, Doc)> {
    rows(si, "items").into_iter().filter_map(|r| Some((id(&r, "fa_asset_disposal")?, r))).collect()
}

pub struct DisposalHook;

#[async_trait]
impl DocHook for DisposalHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if docstatus(doc) != 0 {
            return Ok(());
        }
        let Some(a) = id(doc, "asset") else { return Ok(()) };
        let current = get(hc.kernel, hc.ctx, hc.tx, "Asset", a).await?;
        set(doc, "company", current.get("company").cloned().unwrap_or(Value::Null));
        if id(doc, "cost_center").is_none() {
            set(doc, "cost_center", current.get("cost_center").cloned().unwrap_or(Value::Null));
        }
        // A preview from what is booked now; the submit books the depreciation due by the date.
        let proceeds = if s(doc, "disposal_type") == Some("Sell") { d(doc, "sale_amount") } else { Decimal::ZERO };
        let value = d(&current, "value_after_depreciation");
        set(doc, "gross_amount", money(d(&current, "total_asset_cost")));
        set(doc, "accumulated_depreciation", money(d(&current, "accumulated_depreciation")));
        set(doc, "net_book_value", money(value));
        set(doc, "proceeds", money(proceeds));
        set(doc, "gain_or_loss", money(proceeds - value));
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let a = id(doc, "asset").ok_or_else(|| Error::field("asset", "is required"))?;
        let current = hc.kernel.get_tx(hc.ctx, hc.tx, "Asset", a, false).await?;
        in_service(&current)?;
        if matches!(s(&current, "status"), Some("Out of Order" | "In Maintenance")) {
            return Err(Error::field(
                "asset",
                format!(
                    "asset {} is {}; finish or delete its repair or maintenance first",
                    named(&current),
                    s(&current, "status").unwrap_or_default().to_lowercase()
                ),
            ));
        }
        let on = day(doc, "disposal_date").ok_or_else(|| Error::field("disposal_date", "is required"))?;
        if on > hc.ctx.today() {
            return Err(Error::field("disposal_date", "cannot be in the future"));
        }
        if day(&current, "available_for_use_date").is_some_and(|f| on < f) {
            return Err(Error::field("disposal_date", "is before the asset was available for use"));
        }
        if s(doc, "disposal_type") == Some("Sell") {
            let co = company(hc.kernel, hc.ctx, hc.tx, id(&current, "company").unwrap_or_default()).await?;
            check_places(doc, "sale_amount", co.precision)?;
        }
        Ok(())
    }

    /// A draft sale keeps its draft invoice in step with it.
    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        let (Some(old), Some(si)) = (old, id(doc, "sales_invoice")) else { return Ok(()) };
        if ["sale_amount", "customer", "disposal_date"].iter().all(|f| old.get(*f) == doc.get(*f)) {
            return Ok(());
        }
        let inv = get(hc.kernel, hc.ctx, hc.tx, "SalesInvoice", si).await?;
        if docstatus(&inv) != 0 {
            return Ok(());
        }
        let me = id(doc, "id");
        let items: Vec<Value> = rows(&inv, "items")
            .iter()
            .map(|r| {
                if id(r, "fa_asset_disposal") == me {
                    json!({"id": r.get("id"), "price_list_rate": doc.get("sale_amount")})
                } else {
                    json!({"id": r.get("id")})
                }
            })
            .collect();
        let mut patch = Doc::new();
        patch.insert("items".into(), Value::Array(items));
        patch.insert("customer".into(), doc.get("customer").cloned().unwrap_or(Value::Null));
        patch.insert("posting_date".into(), doc.get("disposal_date").cloned().unwrap_or(Value::Null));
        hc.kernel.update_tx(&elevated(hc.ctx), hc.tx, "SalesInvoice", si, patch, None).await?;
        Ok(())
    }

    async fn before_submit(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if s(doc, "disposal_type") != Some("Sell") {
            return Ok(());
        }
        let refuse = || Error::invalid("a sale is completed by its sales invoice: create it from this disposal and submit it");
        let si = id(doc, "sales_invoice").ok_or_else(refuse)?;
        let inv = get(hc.kernel, hc.ctx, hc.tx, "SalesInvoice", si).await?;
        if docstatus(&inv) != 1 {
            return Err(refuse());
        }
        set(doc, "disposal_date", inv.get("posting_date").cloned().unwrap_or(Value::Null));
        Ok(())
    }

    async fn on_submit(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let me = id(doc, "id").ok_or_else(|| Error::internal("disposal without id"))?;
        let a = id(doc, "asset").ok_or_else(|| Error::field("asset", "is required"))?;
        let on = day(doc, "disposal_date").ok_or_else(|| Error::field("disposal_date", "is required"))?;
        let (mut current, old) = asset::open(hc.kernel, hc.ctx, hc.tx, a).await?;
        in_service(&current)?;
        let co = company(hc.kernel, hc.ctx, hc.tx, id(&current, "company").unwrap_or_default()).await?;
        let snapshot = asset::depreciate_to_disposal(hc.kernel, hc.ctx, hc.tx, &mut current, on, me, co.precision).await?;
        let value = asset::net_book_value(hc.tx, hc.ctx, &current).await?;
        let accumulated = d(&current, "opening_accumulated_depreciation")
            + rows(&current, "schedule").iter().filter(|r| b(r, "booked")).map(|r| d(r, "depreciation_amount")).sum::<Decimal>();
        let cost = value + accumulated;
        let proceeds = match s(doc, "disposal_type") {
            Some("Sell") => {
                let si = id(doc, "sales_invoice").ok_or_else(|| Error::internal("sale without invoice"))?;
                let inv = get(hc.kernel, hc.ctx, hc.tx, "SalesInvoice", si).await?;
                asset_lines(&inv).iter().filter(|(dsp, _)| *dsp == me).map(|(_, r)| d(r, "base_net_amount")).sum()
            }
            _ => Decimal::ZERO,
        };
        let gain = proceeds - value;
        let category = id(&current, "asset_category").ok_or_else(|| Error::internal("asset without category"))?;
        let (_, accounts) = setup::accounts(hc.kernel, hc.ctx, hc.tx, category, co.id).await?;
        let fixed_asset = id(&current, "fixed_asset_account").unwrap_or(accounts.fixed_asset);
        let cc = id(doc, "cost_center").or_else(|| id(&current, "cost_center"));
        let memo = format!("{} of {} {}", s(doc, "disposal_type").unwrap_or("Disposal"), named(&current), s(&current, "asset_name").unwrap_or_default());
        let mut entry = Entry::new(co.id, on, "AssetDisposal", me, s(doc, "number").map(str::to_string));
        entry.memo = Some(memo.clone());
        if !accumulated.is_zero() {
            let acc = match id(&current, "accumulated_depreciation_account") {
                Some(acc) => acc,
                None => accounts.accumulated()?,
            };
            entry.lines.push(Line::new(acc, accumulated, &co.currency_code).cost_center(cc).remarks(memo.clone()));
        }
        // The invoice already credited the proceeds to the fixed asset account.
        entry.lines.push(Line::new(fixed_asset, -(cost - proceeds), &co.currency_code).cost_center(cc).remarks(memo.clone()));
        let mut disposal_account = Value::Null;
        if !gain.is_zero() {
            let acc = accounts.disposal()?;
            disposal_account = json!(acc.to_string());
            entry.lines.push(Line::new(acc, -gain, &co.currency_code).cost_center(cc).remarks(if gain > Decimal::ZERO {
                "Gain on disposal"
            } else {
                "Loss on disposal"
            }));
        }
        ledger::post(hc.tx, hc.ctx, entry, co.precision).await?;
        let mut v = Doc::new();
        v.insert("gross_amount".into(), money(cost));
        v.insert("accumulated_depreciation".into(), money(accumulated));
        v.insert("net_book_value".into(), money(value));
        v.insert("proceeds".into(), money(proceeds));
        v.insert("gain_or_loss".into(), money(gain));
        v.insert("disposal_account".into(), disposal_account);
        v.insert("schedule_snapshot".into(), snapshot);
        if s(doc, "disposal_type") == Some("Sell") {
            v.insert("sale_amount".into(), money(proceeds));
        }
        hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "AssetDisposal", me, v).await?;
        set(&mut current, "disposal", me.to_string());
        set(&mut current, "disposal_date", on.to_string());
        asset::carry_dates(hc.tx, hc.ctx.tenant, &mut current, false).await?;
        asset::save(hc.kernel, hc.ctx, hc.tx, current, &old, None).await?;
        Ok(())
    }

    async fn before_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let Some(si) = id(doc, "sales_invoice").filter(|_| s(doc, "disposal_type") == Some("Sell")) {
            let inv = get(hc.kernel, hc.ctx, hc.tx, "SalesInvoice", si).await?;
            if docstatus(&inv) == 1 {
                return Err(Error::invalid(format!("cancel sales invoice {}; that cancels this disposal", named(&inv))));
            }
        }
        Ok(())
    }

    /// A draft sale's invoice goes first (deleting it frees the disposal).
    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if let Some(si) = id(doc, "sales_invoice").filter(|_| docstatus(doc) == 0) {
            let inv = get(hc.kernel, hc.ctx, hc.tx, "SalesInvoice", si).await?;
            if docstatus(&inv) == 0 {
                return Err(Error::invalid(format!("delete draft sales invoice {} first", named(&inv))));
            }
        }
        Ok(())
    }

    async fn on_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let me = id(doc, "id").ok_or_else(|| Error::internal("disposal without id"))?;
        let a = id(doc, "asset").ok_or_else(|| Error::field("asset", "is required"))?;
        let (mut current, old) = asset::open(hc.kernel, hc.ctx, hc.tx, a).await?;
        let co = company(hc.kernel, hc.ctx, hc.tx, id(&current, "company").unwrap_or_default()).await?;
        ledger::reverse(hc.tx, hc.ctx, "AssetDisposal", me, hc.ctx.today(), co.precision).await?;
        let snapshot = doc.get("schedule_snapshot").cloned().unwrap_or(Value::Null);
        asset::restore_after_disposal(hc.kernel, hc.ctx, hc.tx, &mut current, me, &snapshot).await?;
        set(&mut current, "disposal", Value::Null);
        set(&mut current, "disposal_date", Value::Null);
        asset::carry_dates(hc.tx, hc.ctx.tenant, &mut current, true).await?;
        asset::save(hc.kernel, hc.ctx, hc.tx, current, &old, None).await?;
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        if name != "make_sales_invoice" {
            return Ok(None);
        }
        let me = id(doc, "id").ok_or_else(|| Error::internal("disposal without id"))?;
        if let Some(si) = id(doc, "sales_invoice") {
            let inv = get(hc.kernel, hc.ctx, hc.tx, "SalesInvoice", si).await?;
            match docstatus(&inv) {
                0 => return Ok(Some(created("SalesInvoice", &inv))),
                1 => return Err(Error::invalid(format!("sales invoice {} already sells this asset", named(&inv)))),
                _ => {}
            }
        }
        let a = id(doc, "asset").ok_or_else(|| Error::field("asset", "is required"))?;
        let current = get(hc.kernel, hc.ctx, hc.tx, "Asset", a).await?;
        in_service(&current)?;
        let customer = id(doc, "customer").ok_or_else(|| Error::field("customer", "is required"))?;
        if d(doc, "sale_amount") <= Decimal::ZERO {
            return Err(Error::field("sale_amount", "enter the sale amount"));
        }
        let co = company(hc.kernel, hc.ctx, hc.tx, id(&current, "company").unwrap_or_default()).await?;
        let input = json!({
            "company": co.id.to_string(), "customer": customer.to_string(), "posting_date": doc.get("disposal_date"), "currency": co.currency_id.to_string(),
            "cost_center": doc.get("cost_center"), "remarks": format!("Sale of asset {}", named(&current)),
            "items": [{
                "item": current.get("item"), "qty": "1", "price_list_rate": doc.get("sale_amount"), "fa_asset_disposal": me.to_string(),
                "income_account": current.get("fixed_asset_account"), "cost_center": doc.get("cost_center"),
                "description": format!("{} {}", named(&current), s(&current, "asset_name").unwrap_or_default()),
            }],
        });
        let Value::Object(input) = input else { return Err(Error::internal("invoice input")) };
        let inv = create_for_user(hc, "SalesInvoice", input).await?;
        let mut v = Doc::new();
        v.insert("sales_invoice".into(), inv.get("id").cloned().unwrap_or(Value::Null));
        hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "AssetDisposal", me, v).await?;
        Ok(Some(created("SalesInvoice", &inv)))
    }
}

pub struct SalesInvoiceHook;

#[async_trait]
impl DocHook for SalesInvoiceHook {
    /// An asset line is always credited to the asset's fixed asset account.
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if !installed(hc.ctx) || docstatus(doc) != 0 || asset_lines(doc).is_empty() {
            return Ok(());
        }
        let mut items = rows(doc, "items");
        for r in items.iter_mut() {
            let Some(dsp) = id(r, "fa_asset_disposal") else { continue };
            let disposal = get(hc.kernel, hc.ctx, hc.tx, "AssetDisposal", dsp).await?;
            let current = get(hc.kernel, hc.ctx, hc.tx, "Asset", id(&disposal, "asset").unwrap_or_default()).await?;
            if let Some(acc) = id(&current, "fixed_asset_account") {
                set(r, "income_account", acc.to_string());
            }
        }
        set(doc, "items", Value::Array(items.into_iter().map(Value::Object).collect()));
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if !installed(hc.ctx) {
            return Ok(());
        }
        if b(doc, "is_return")
            && let Some(orig) = id(doc, "return_against")
        {
            let inv = get(hc.kernel, hc.ctx, hc.tx, "SalesInvoice", orig).await?;
            if !asset_lines(&inv).is_empty() {
                return Err(Error::field("return_against", "an asset sale is undone by cancelling its invoice, not by a credit note"));
            }
        }
        let lines = asset_lines(doc);
        if lines.is_empty() {
            return Ok(());
        }
        let me = id(doc, "id");
        let mut errors = Vec::new();
        let mut seen = HashSet::new();
        for (i, r) in rows(doc, "items").iter().enumerate() {
            let Some(dsp) = id(r, "fa_asset_disposal") else { continue };
            let err = |f: &str, m: &str| FieldError::new(f, m).in_row("items", i);
            if !seen.insert(dsp) {
                errors.push(err("fa_asset_disposal", "this disposal is on the invoice twice"));
                continue;
            }
            let disposal = get(hc.kernel, hc.ctx, hc.tx, "AssetDisposal", dsp).await?;
            if docstatus(&disposal) == 2 {
                errors.push(err("fa_asset_disposal", "this sale's disposal was cancelled; sell the asset through a new disposal"));
            } else if docstatus(&disposal) != 0 || s(&disposal, "disposal_type") != Some("Sell") {
                errors.push(err("fa_asset_disposal", "must be a draft sale"));
            } else if id(&disposal, "company") != id(doc, "company") || id(&disposal, "customer") != id(doc, "customer") {
                errors.push(err("fa_asset_disposal", "the disposal sells to another customer or company"));
            } else if id(&disposal, "sales_invoice").is_some_and(|x| Some(x) != me) {
                errors.push(err("fa_asset_disposal", "another invoice sells this asset"));
            }
            if d(r, "qty") != Decimal::ONE || d(r, "discount_percent") > Decimal::ZERO {
                errors.push(err("qty", "an asset is sold whole: quantity 1, no line discount"));
            }
        }
        if !errors.is_empty() {
            return Err(Error::Validation(errors));
        }
        if b(doc, "is_return") {
            return Err(Error::field("is_return", "a credit note cannot sell an asset"));
        }
        let cid = id(doc, "company").ok_or_else(|| Error::field("company", "is required"))?;
        let co = company(hc.kernel, hc.ctx, hc.tx, cid).await?;
        if id(doc, "currency").is_some_and(|c| c != co.currency_id) {
            return Err(Error::field("currency", "assets are sold in the company currency"));
        }
        Ok(())
    }

    async fn on_submit(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if !installed(hc.ctx) {
            return Ok(());
        }
        let engine = elevated(hc.ctx);
        for (dsp, _) in asset_lines(doc) {
            let mut v = Doc::new();
            v.insert("sales_invoice".into(), doc.get("id").cloned().unwrap_or(Value::Null));
            v.insert("disposal_date".into(), doc.get("posting_date").cloned().unwrap_or(Value::Null));
            hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "AssetDisposal", dsp, v).await?;
            hc.kernel.submit_tx(&engine, hc.tx, "AssetDisposal", dsp, None).await?;
        }
        Ok(())
    }

    async fn on_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if !installed(hc.ctx) {
            return Ok(());
        }
        let engine = elevated(hc.ctx);
        for (dsp, _) in asset_lines(doc) {
            let disposal = get(hc.kernel, hc.ctx, hc.tx, "AssetDisposal", dsp).await?;
            if docstatus(&disposal) == 1 {
                hc.kernel.cancel_tx(&engine, hc.tx, "AssetDisposal", dsp, None).await?;
            }
        }
        Ok(())
    }

    /// Deleting a draft invoice frees its disposals for a new one.
    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if !installed(hc.ctx) {
            return Ok(());
        }
        for (dsp, _) in asset_lines(doc) {
            let disposal = get(hc.kernel, hc.ctx, hc.tx, "AssetDisposal", dsp).await?;
            if id(&disposal, "sales_invoice") == id(doc, "id") && docstatus(&disposal) == 0 {
                let mut v = Doc::new();
                v.insert("sales_invoice".into(), Value::Null);
                hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "AssetDisposal", dsp, v).await?;
            }
        }
        Ok(())
    }
}

/// Depreciation entries belong to their asset's schedule: they are reversed by cancelling the
/// asset or its disposal (or corrected by a value adjustment), never one by one.
pub struct JournalEntryGuard;

#[async_trait]
impl DocHook for JournalEntryGuard {
    async fn before_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if !installed(hc.ctx) {
            return Ok(());
        }
        let Some(je) = id(doc, "id") else { return Ok(()) };
        let asset: Option<String> = sqlx::query_scalar(
            "select coalesce(a.number, a.id::text) from e_asset__schedule r join e_asset a on a.tenant_id = r.tenant_id and a.id = r.parent_id \
             where r.tenant_id = $1 and r.journal_entry = $2 limit 1",
        )
        .bind(hc.ctx.tenant)
        .bind(je)
        .fetch_optional(&mut **hc.tx)
        .await?;
        if let Some(n) = asset {
            return Err(Error::invalid(format!(
                "this entry books depreciation of asset {n}; it is reversed by cancelling the asset or its disposal (use a value adjustment to correct the asset's value)"
            )));
        }
        Ok(())
    }
}
