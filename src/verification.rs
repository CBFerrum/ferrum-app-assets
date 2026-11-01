//! Physical verification: someone walked up to an asset and saw it, or looked and could not find
//! it. Each submitted verification is the record of that. The asset carries the latest standing
//! one (when, what it found, when the next is due), recomputed from the verifications that stand,
//! so cancelling one takes it back.

use crate::asset::{DATES, carry_dates, day, in_service};
use crate::int;
use crate::schedule::add_months;
use async_trait::async_trait;
use chrono::NaiveDate;
use ferrum_domain::util::{get, id, s, set, sys};
use ferrum_kernel::{Doc, DocHook, Error, HookCtx, Result, Tx};
use serde_json::Value;
use uuid::Uuid;

/// The check after this one: the category's interval from the day it was done.
pub(crate) fn next_check(done: NaiveDate, interval_months: i64) -> Option<NaiveDate> {
    (1..=120).contains(&interval_months).then(|| add_months(done, interval_months as i32))
}

pub struct VerificationHook;

#[async_trait]
impl DocHook for VerificationHook {
    /// The company follows the asset, the person is whoever is signed in, and a found asset's next
    /// check follows its category unless one is given.
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if doc.get("docstatus").and_then(Value::as_i64).unwrap_or(0) != 0 {
            return Ok(());
        }
        let Some(a) = id(doc, "asset") else { return Ok(()) };
        let current = get(hc.kernel, hc.ctx, hc.tx, "Asset", a).await?;
        set(doc, "company", current.get("company").cloned().unwrap_or(Value::Null));
        if !hc.ctx.is_system {
            set(doc, "verified_by", hc.ctx.email.to_lowercase());
        }
        if s(doc, "result") == Some("Found") && s(doc, "next_verification_date").is_none() {
            let (Some(on), Some(category)) = (day(doc, "verified_on"), id(&current, "asset_category")) else { return Ok(()) };
            let category = get(hc.kernel, hc.ctx, hc.tx, "AssetCategory", category).await?;
            if let Some(next) = next_check(on, int(&category, "verification_interval_months")) {
                set(doc, "next_verification_date", next.to_string());
            }
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let a = id(doc, "asset").ok_or_else(|| Error::field("asset", "is required"))?;
        // The caller must be able to see the asset they check.
        let current = hc.kernel.get_tx(hc.ctx, hc.tx, "Asset", a, false).await.map_err(|e| match e {
            Error::NotFound(_) => Error::field("asset", "not found"),
            other => other,
        })?;
        in_service(&current)?;
        let on = day(doc, "verified_on").ok_or_else(|| Error::field("verified_on", "is required"))?;
        if on > hc.ctx.today() {
            return Err(Error::field("verified_on", "cannot be in the future"));
        }
        if day(&current, "purchase_date").is_some_and(|p| on < p) {
            return Err(Error::field("verified_on", "is before the asset was bought"));
        }
        if day(doc, "next_verification_date").is_some_and(|n| n <= on) {
            return Err(Error::field("next_verification_date", "must be after the day it was checked"));
        }
        if s(doc, "result") == Some("Not Found") && (id(doc, "seen_at").is_some() || s(doc, "next_verification_date").is_some()) {
            return Err(Error::field("result", "an asset that was not found was seen nowhere and has no next check"));
        }
        Ok(())
    }

    async fn on_submit(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        refresh(hc, doc).await
    }

    async fn on_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        refresh(hc, doc).await
    }
}

/// The latest verification that stands: the day, what it found and when the next is due.
pub(crate) type Standing = (NaiveDate, String, Option<NaiveDate>);

pub(crate) async fn standing(tx: &mut Tx, tenant: Uuid, asset: Uuid) -> Result<Option<Standing>> {
    Ok(sqlx::query_as(
        "select verified_on, result, next_verification_date from e_asset_verification where tenant_id = $1 and asset = $2 and docstatus = 1 \
         order by verified_on desc, created_at desc limit 1",
    )
    .bind(tenant)
    .bind(asset)
    .fetch_optional(&mut **tx)
    .await?)
}

/// The asset carries what its standing verification says, and what its movements say it is due.
async fn refresh(hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
    let Some(a) = id(doc, "asset") else { return Ok(()) };
    let mut current = get(hc.kernel, hc.ctx, hc.tx, "Asset", a).await?;
    let live = in_service(&current).is_ok();
    carry_dates(hc.tx, hc.ctx.tenant, &mut current, live).await?;
    let v: Doc = DATES.iter().map(|k| (k.to_string(), current.get(*k).cloned().unwrap_or(Value::Null))).collect();
    hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "Asset", a, v).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_check_follows_the_category() {
        let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
        assert_eq!(next_check(d(2024, 5, 1), 6), Some(d(2024, 11, 1)));
        assert_eq!(next_check(d(2024, 8, 31), 6), Some(d(2025, 2, 28)), "a month end stays a month end");
        assert_eq!(next_check(d(2024, 5, 1), 0), None, "no interval, no routine check");
        assert_eq!(next_check(d(2024, 5, 1), 121), None);
    }
}
