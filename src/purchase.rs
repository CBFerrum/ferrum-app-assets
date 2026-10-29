//! Capitalization from purchase invoices: a line marked "is fixed asset" is booked to its
//! category's fixed asset account (or capital work in progress), and submitting the invoice drafts
//! the assets — one per unit, or one grouped asset when the category says so — carrying the
//! line's amount exactly. Cancelling the invoice removes the drafts and is refused while any of
//! its assets is submitted.

use crate::installed;
use crate::schedule::spread;
use crate::setup;
use async_trait::async_trait;
use ferrum_domain::util::{b, company, d, elevated, get, id, money, rows, set};
use ferrum_kernel::{Doc, DocHook, Error, FieldError, HookCtx, Result};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

/// Units one invoice line may turn into separate assets (beyond that, group them).
const MAX_UNITS: i64 = 1000;

pub struct PurchaseInvoiceHook;

#[async_trait]
impl DocHook for PurchaseInvoiceHook {
    /// Runs after the platform priced the invoice: marks lines of fixed asset items and books them
    /// to the category's account (CWIP when the category uses it).
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if !installed(hc.ctx) || doc.get("docstatus").and_then(Value::as_i64).unwrap_or(0) != 0 {
            return Ok(());
        }
        let Some(cid) = id(doc, "company") else { return Ok(()) };
        let mut items = rows(doc, "items");
        let mut cache: HashMap<Uuid, Doc> = HashMap::new();
        for r in items.iter_mut() {
            let Some(item) = id(r, "item") else { continue };
            if let std::collections::hash_map::Entry::Vacant(e) = cache.entry(item) {
                e.insert(get(hc.kernel, hc.ctx, hc.tx, "Item", item).await?);
            }
            let it = &cache[&item];
            if b(it, "fa_is_fixed_asset") {
                set(r, "fa_is_fixed_asset", true);
                if id(r, "fa_asset_category").is_none()
                    && let Some(c) = id(it, "fa_asset_category")
                {
                    set(r, "fa_asset_category", c.to_string());
                }
            }
            if !b(r, "fa_is_fixed_asset") {
                continue;
            }
            let Some(category) = id(r, "fa_asset_category") else { continue };
            let (cat, accounts) = setup::accounts(hc.kernel, hc.ctx, hc.tx, category, cid).await?;
            let account = if b(&cat, "enable_cwip_accounting") { accounts.cwip()? } else { accounts.fixed_asset };
            set(r, "expense_account", account.to_string());
        }
        set(doc, "items", Value::Array(items.into_iter().map(Value::Object).collect()));
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if !installed(hc.ctx) {
            return Ok(());
        }
        let mut errors = Vec::new();
        for (i, r) in rows(doc, "items").iter().enumerate() {
            if !b(r, "fa_is_fixed_asset") {
                continue;
            }
            if b(doc, "is_return") {
                errors.push(
                    FieldError::new("fa_is_fixed_asset", "fixed assets are not returned with a debit note: cancel the invoice, or dispose of the asset")
                        .in_row("items", i),
                );
                continue;
            }
            if let Some(item) = id(r, "item") {
                let it = get(hc.kernel, hc.ctx, hc.tx, "Item", item).await?;
                if !b(&it, "fa_is_fixed_asset") || b(&it, "is_stock_item") {
                    errors.push(FieldError::new("item", "only fixed asset items (not stock items) can be bought as assets").in_row("items", i));
                }
            }
            let qty = d(r, "qty");
            if qty <= Decimal::ZERO || !qty.fract().is_zero() {
                errors.push(FieldError::new("qty", "assets are bought in whole units").in_row("items", i));
            } else if qty > Decimal::from(MAX_UNITS) {
                errors.push(FieldError::new("qty", format!("more than {MAX_UNITS} units: use a category that groups assets")).in_row("items", i));
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) }
    }

    async fn on_submit(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if !installed(hc.ctx) {
            return Ok(());
        }
        let pi = id(doc, "id").ok_or_else(|| Error::internal("invoice without id"))?;
        let cid = id(doc, "company").ok_or_else(|| Error::field("company", "is required"))?;
        let co = company(hc.kernel, hc.ctx, hc.tx, cid).await?;
        let engine = elevated(hc.ctx);
        for r in rows(doc, "items").iter().filter(|r| b(r, "fa_is_fixed_asset")) {
            let category = id(r, "fa_asset_category").ok_or_else(|| Error::field("fa_asset_category", "is required"))?;
            let cat = get(hc.kernel, hc.ctx, hc.tx, "AssetCategory", category).await?;
            let qty = d(r, "qty");
            let amount = d(r, "base_net_amount");
            let grouped = b(&cat, "group_assets");
            let whole = qty.trunc().to_i64().unwrap_or(1).max(1);
            let units = if grouped { 1 } else { whole as usize };
            // One asset per unit carries an equal share; the last absorbs the rounding.
            let shares = spread(amount, &vec![Decimal::ONE; units], co.precision);
            for share in shares {
                let input = json!({
                    "asset_name": r.get("item_name"), "item": r.get("item"), "asset_category": category.to_string(), "company": cid.to_string(),
                    "location": r.get("fa_asset_location"), "cost_center": r.get("cost_center"), "purchase_date": doc.get("posting_date"),
                    "available_for_use_date": doc.get("posting_date"), "gross_purchase_amount": money(share),
                    "asset_quantity": if grouped { whole } else { 1 },
                    "purchase_invoice": pi.to_string(), "purchase_invoice_item": r.get("id"),
                });
                let Value::Object(input) = input else { continue };
                hc.kernel.insert_tx(&engine, hc.tx, "Asset", input).await?;
            }
        }
        Ok(())
    }

    async fn before_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if !installed(hc.ctx) {
            return Ok(());
        }
        let pi = id(doc, "id").ok_or_else(|| Error::internal("invoice without id"))?;
        let live: Vec<String> = sqlx::query_scalar(
            "select coalesce(number, id::text) from e_asset where tenant_id = $1 and purchase_invoice = $2 and docstatus = 1 order by number limit 5",
        )
        .bind(hc.ctx.tenant)
        .bind(pi)
        .fetch_all(&mut **hc.tx)
        .await?;
        if !live.is_empty() {
            return Err(Error::invalid(format!("assets made from this invoice are submitted ({}); cancel them first", live.join(", "))));
        }
        Ok(())
    }

    async fn on_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if !installed(hc.ctx) {
            return Ok(());
        }
        let pi = id(doc, "id").ok_or_else(|| Error::internal("invoice without id"))?;
        let drafts: Vec<Uuid> = sqlx::query_scalar("select id from e_asset where tenant_id = $1 and purchase_invoice = $2 and docstatus = 0")
            .bind(hc.ctx.tenant)
            .bind(pi)
            .fetch_all(&mut **hc.tx)
            .await?;
        let engine = elevated(hc.ctx);
        for asset in drafts {
            hc.kernel.delete_tx(&engine, hc.tx, "Asset", asset, None).await?;
        }
        Ok(())
    }
}
