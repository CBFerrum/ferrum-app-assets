//! Assets: fixed assets with categories, depreciation schedules posted to the ledger, movements
//! and custody, value adjustments, repairs, maintenance plans, sale and scrapping, and
//! capitalization from purchase invoices.
//!
//! An `Asset` is submitted with its depreciation schedule; `maintenance` books the rows that fall
//! due (each as a submitted depreciation journal entry), and every document that changes the
//! asset's value (value adjustments, capitalized repairs, disposals) first books the depreciation
//! due by its date and then re-plans the rest of the schedule. The asset's figures (cost,
//! accumulated depreciation, net book value, status) are always recomputed from those documents,
//! never accumulated, so they cannot drift.

mod asset;
mod disposal;
mod maintenance;
mod movement;
mod periodic;
mod purchase;
mod reports;
pub mod schedule;
mod setup;
mod value;

use ferrum_domain::reports::ReportSource;
use ferrum_domain::util::{d, s};
use ferrum_kernel::{Ctx, Doc, Error, HookRegistry, Kernel, Result};
use serde_json::Value;
use std::sync::Arc;

/// Registers this package's document hooks. Hooks on platform entities (company, item, invoices,
/// journal entries) run after the platform's own and do nothing where the package is not installed.
pub fn register(h: &mut HookRegistry) {
    h.register("Company", Arc::new(setup::CompanyHook))
        .register("Item", Arc::new(setup::ItemHook))
        .register("AssetCategory", Arc::new(setup::CategoryHook))
        .register("Location", Arc::new(setup::LocationHook))
        .register("Asset", Arc::new(asset::AssetHook))
        .register("PurchaseInvoice", Arc::new(purchase::PurchaseInvoiceHook))
        .register("AssetMovement", Arc::new(movement::MovementHook))
        .register("AssetValueAdjustment", Arc::new(value::AdjustmentHook))
        .register("AssetRepair", Arc::new(value::RepairHook))
        .register("AssetMaintenance", Arc::new(maintenance::PlanHook))
        .register("AssetMaintenanceLog", Arc::new(maintenance::LogHook))
        .register("AssetDisposal", Arc::new(disposal::DisposalHook))
        .register("SalesInvoice", Arc::new(disposal::SalesInvoiceHook))
        .register("JournalEntry", Arc::new(disposal::JournalEntryGuard));
}

/// This package's reports.
pub fn reports() -> Vec<Arc<dyn ReportSource>> {
    vec![Arc::new(reports::AssetReports)]
}

/// Periodic upkeep: every minute, on one replica at a time (advisory lock); idempotent and cheap.
/// Books the depreciation that fell due (up to each workspace's today) and opens the maintenance
/// tasks that came due, notifying their assignees.
pub async fn maintenance(k: &Kernel) -> Result<()> {
    periodic::run(k).await
}

/// Whether the workspace has this package (hooks on platform entities run for every workspace).
pub(crate) fn installed(ctx: &Ctx) -> bool {
    ctx.schema.entity("Asset").is_some()
}

/// A whole-number field (missing or empty reads as 0).
pub(crate) fn int(doc: &Doc, k: &str) -> i64 {
    doc.get(k).and_then(Value::as_i64).unwrap_or(0)
}

/// An error naming a document by its number (or its id while it has none).
pub(crate) fn named(doc: &Doc) -> String {
    s(doc, "number").or_else(|| s(doc, "id")).unwrap_or_default().to_string()
}

/// Rejects a decimal with more places than the currency allows (amounts are stored exactly).
pub(crate) fn check_places(doc: &Doc, field: &str, precision: u32) -> Result<()> {
    let v = d(doc, field);
    if v.normalize().scale() > precision {
        return Err(Error::field(field, format!("use at most {precision} decimal places")));
    }
    Ok(())
}

include!(concat!(env!("OUT_DIR"), "/packages.rs"));

/// This app's manifest (`app.yaml`).
pub const MANIFEST: &str = include_str!("../app.yaml");

/// This app's metadata packages, embedded at build time.
pub fn packages() -> std::result::Result<Vec<ferrum_meta::Package>, ferrum_meta::MetaError> {
    ferrum_meta::catalog::load_embedded(EMBEDDED)
}

#[cfg(test)]
mod tests {
    #[test]
    fn packages_form_a_valid_catalog_with_the_platform() {
        let mut pkgs = ferrum_domain::packages().expect("platform packages");
        pkgs.extend(ferrum_app_erp::packages().expect("ERP's packages"));
        pkgs.extend(super::packages().expect("app packages parse"));
        let cat = ferrum_meta::Catalog::from_packages(pkgs).expect("they validate together");
        let all: Vec<String> = cat.packages().map(|p| p.name().to_string()).collect();
        ferrum_meta::effective::build(&cat, &all, &[], 1).expect("effective schema with every package");
    }
}
