//! Asset movements: transfers between locations, issues to custodians and receipts back. The
//! submitted movements are the custody history; the asset carries where it is and who has it.
//! A movement is undone (cancelled) only while no later movement has moved its assets again.
//! An issue may say when the custodian is to hand the asset back; the asset carries the date of
//! the issue that still stands, worked out from the movements (`due_back`), never accumulated.

use crate::asset::{self, day, in_service};
use crate::named;
use async_trait::async_trait;
use chrono::NaiveDate;
use ferrum_domain::util::{get, id, rows, s, set, sys};
use ferrum_kernel::{Doc, DocHook, Error, FieldError, HookCtx, Result, Tx};
use serde_json::{Value, json};
use std::collections::HashSet;
use uuid::Uuid;

/// Where a row leaves the asset and who holds it then. Transfer keeps the custodian unless a new
/// one is given, Issue hands it to one, Receipt takes it back from them; the location stays
/// where it is unless a target is given.
fn effective(purpose: &str, row: &Doc) -> (Option<Uuid>, Option<String>) {
    let target = id(row, "target_location").or_else(|| id(row, "source_location"));
    let to = match purpose {
        "Issue" => s(row, "to_custodian"),
        "Receipt" => None,
        _ => s(row, "to_custodian").or_else(|| s(row, "from_custodian")),
    };
    (target, to.map(str::to_lowercase))
}

fn custodian(doc: &Doc, k: &str) -> Option<String> {
    s(doc, k).map(str::to_lowercase)
}

fn put(doc: &mut Doc, list: Vec<Doc>) {
    set(doc, "assets", Value::Array(list.into_iter().map(Value::Object).collect()));
}

/// One standing movement of an asset: purpose, the date it asked for, who held it before and after.
type Step = (String, Option<NaiveDate>, Option<String>, Option<String>);

/// When the asset is due back: what the last issue asked for, until the custody changes hands
/// again (a receipt, or a transfer to another person), oldest movement first.
fn fold_due(history: &[Step]) -> Option<NaiveDate> {
    let who = |c: &Option<String>| c.as_deref().map(str::to_lowercase);
    history.iter().fold(None, |due, (purpose, date, from, to)| match purpose.as_str() {
        "Issue" => *date,
        _ if who(from) == who(to) => due,
        _ => None,
    })
}

/// The date the asset is due back, from the movements that stand (a cancelled one counts for
/// nothing).
pub(crate) async fn due_back(tx: &mut Tx, tenant: Uuid, asset: Uuid) -> Result<Option<NaiveDate>> {
    let history: Vec<Step> = sqlx::query_as(
        "select m.purpose, i.expected_return_date, i.from_custodian, i.to_custodian from e_asset_movement__assets i \
         join e_asset_movement m on m.tenant_id = i.tenant_id and m.id = i.parent_id \
         where i.tenant_id = $1 and i.asset = $2 and m.docstatus = 1 order by m.transaction_date, m.created_at, i.idx",
    )
    .bind(tenant)
    .bind(asset)
    .fetch_all(&mut **tx)
    .await?;
    Ok(fold_due(&history))
}

pub struct MovementHook;

#[async_trait]
impl DocHook for MovementHook {
    /// While a draft, each row shows where its asset is and who has it.
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if doc.get("docstatus").and_then(Value::as_i64).unwrap_or(0) != 0 {
            return Ok(());
        }
        let mut list = rows(doc, "assets");
        for r in list.iter_mut() {
            let Some(a) = id(r, "asset") else { continue };
            let current = get(hc.kernel, hc.ctx, hc.tx, "Asset", a).await?;
            set(r, "source_location", current.get("location").cloned().unwrap_or(Value::Null));
            set(r, "from_custodian", current.get("custodian").cloned().unwrap_or(Value::Null));
        }
        put(doc, list);
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let purpose = s(doc, "purpose").unwrap_or("Transfer").to_string();
        let on = day(doc, "transaction_date").ok_or_else(|| Error::field("transaction_date", "is required"))?;
        if on > hc.ctx.today() {
            return Err(Error::field("transaction_date", "cannot be in the future"));
        }
        let mut errors = Vec::new();
        let mut seen = HashSet::new();
        for (i, r) in rows(doc, "assets").iter().enumerate() {
            let Some(a) = id(r, "asset") else { continue };
            let err = |f: &str, m: String| FieldError::new(f, m).in_row("assets", i);
            if !seen.insert(a) {
                errors.push(err("asset", "is listed twice".into()));
                continue;
            }
            // The caller must be able to see the asset they move.
            let current = hc.kernel.get_tx(hc.ctx, hc.tx, "Asset", a, false).await?;
            if let Err(Error::Validation(e)) = in_service(&current) {
                errors.extend(e.into_iter().map(|f| f.in_row("assets", i)));
                continue;
            }
            if id(&current, "company") != id(doc, "company") {
                errors.push(err("asset", format!("asset {} belongs to another company", named(&current))));
            }
            if day(&current, "purchase_date").is_some_and(|p| on < p) {
                errors.push(err("asset", format!("asset {} was bought after this date", named(&current))));
            }
            let latest: Option<NaiveDate> = sqlx::query_scalar(
                "select max(m.transaction_date) from e_asset_movement m join e_asset_movement__assets i on i.tenant_id = m.tenant_id and i.parent_id = m.id \
                 where m.tenant_id = $1 and i.asset = $2 and m.docstatus = 1 and m.id is distinct from $3",
            )
            .bind(hc.ctx.tenant)
            .bind(a)
            .bind(id(doc, "id"))
            .fetch_one(&mut **hc.tx)
            .await?;
            if latest.is_some_and(|l| on < l) {
                errors.push(err("asset", format!("asset {} moved later than this date already", named(&current))));
            }
            if let Some(due) = day(r, "expected_return_date") {
                if purpose != "Issue" {
                    errors.push(err("expected_return_date", "only an issue has a return date".into()));
                } else if due < on {
                    errors.push(err("expected_return_date", "cannot be before the day it is handed out".into()));
                }
            }
            let (target, to) = effective(&purpose, r);
            match purpose.as_str() {
                "Transfer" if id(r, "target_location").is_none() || target == id(r, "source_location") => {
                    errors.push(err("target_location", "a transfer needs a different location".into()))
                }
                "Issue" if to.is_none() || to == custodian(r, "from_custodian") => {
                    errors.push(err("to_custodian", "an issue needs the custodian it goes to".into()))
                }
                "Receipt" if id(r, "target_location").is_none() => errors.push(err("target_location", "a receipt needs the location it comes back to".into())),
                "Receipt" if s(r, "to_custodian").is_some() => errors.push(err("to_custodian", "a receipt takes the asset back from its custodian".into())),
                "Receipt" if custodian(r, "from_custodian").is_none() => errors.push(err("asset", "nobody holds this asset".into())),
                _ => {}
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) }
    }

    /// The rows record the custody they leave, so cancelling knows what to restore.
    async fn before_submit(&self, _hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        let purpose = s(doc, "purpose").unwrap_or("Transfer").to_string();
        let mut list = rows(doc, "assets");
        for r in list.iter_mut() {
            let (target, to) = effective(&purpose, r);
            set(r, "target_location", json!(target.map(|t| t.to_string())));
            set(r, "to_custodian", json!(to));
        }
        put(doc, list);
        Ok(())
    }

    async fn on_submit(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        for r in rows(doc, "assets") {
            let Some(a) = id(&r, "asset") else { continue };
            let (current, _) = asset::open(hc.kernel, hc.ctx, hc.tx, a).await?;
            if id(&current, "location") != id(&r, "source_location") || custodian(&current, "custodian") != custodian(&r, "from_custodian") {
                return Err(Error::Conflict(format!("asset {} moved while this movement was being submitted; try again", named(&current))));
            }
            let mut v = Doc::new();
            v.insert("location".into(), r.get("target_location").cloned().unwrap_or(Value::Null));
            v.insert("custodian".into(), r.get("to_custodian").cloned().unwrap_or(Value::Null));
            if in_service(&current).is_ok() {
                v.insert("expected_return_date".into(), json!(due_back(hc.tx, hc.ctx.tenant, a).await?.map(|d| d.to_string())));
            }
            hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "Asset", a, v).await?;
        }
        Ok(())
    }

    async fn on_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        for r in rows(doc, "assets").into_iter().rev() {
            let Some(a) = id(&r, "asset") else { continue };
            let (current, _) = asset::open(hc.kernel, hc.ctx, hc.tx, a).await?;
            if id(&current, "location") != id(&r, "target_location") || custodian(&current, "custodian") != custodian(&r, "to_custodian") {
                return Err(Error::invalid(format!("asset {} was moved again after this movement; cancel the later movement first", named(&current))));
            }
            let mut v = Doc::new();
            v.insert("location".into(), r.get("source_location").cloned().unwrap_or(Value::Null));
            v.insert("custodian".into(), r.get("from_custodian").cloned().unwrap_or(Value::Null));
            if in_service(&current).is_ok() {
                v.insert("expected_return_date".into(), json!(due_back(hc.tx, hc.ctx.tenant, a).await?.map(|d| d.to_string())));
            }
            hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "Asset", a, v).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(purpose: &str, due: Option<&str>, from: Option<&str>, to: Option<&str>) -> Step {
        (purpose.into(), due.map(|d| d.parse().unwrap()), from.map(Into::into), to.map(Into::into))
    }

    #[test]
    fn an_asset_is_due_back_until_its_custody_changes_hands() {
        let day = |d: &str| Some(d.parse::<NaiveDate>().unwrap());
        assert_eq!(fold_due(&[]), None);
        let issue = step("Issue", Some("2024-04-01"), None, Some("bob@acme.test"));
        assert_eq!(fold_due(std::slice::from_ref(&issue)), day("2024-04-01"));
        let moved = step("Transfer", None, Some("Bob@acme.test"), Some("bob@acme.test"));
        assert_eq!(fold_due(&[issue.clone(), moved]), day("2024-04-01"), "the same person in another room: still due");
        let handed_on = step("Transfer", None, Some("bob@acme.test"), Some("alice@acme.test"));
        assert_eq!(fold_due(&[issue.clone(), handed_on]), None, "a new custodian inherits no date");
        let back = step("Receipt", None, Some("bob@acme.test"), None);
        assert_eq!(fold_due(&[issue.clone(), back.clone()]), None, "handed back");
        let again = step("Issue", Some("2024-06-01"), None, Some("bob@acme.test"));
        assert_eq!(fold_due(&[issue, back, again]), day("2024-06-01"), "the last issue decides");
        assert_eq!(fold_due(&[step("Issue", None, None, Some("bob@acme.test"))]), None, "an issue with no date is due whenever");
    }
}
