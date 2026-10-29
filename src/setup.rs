//! Company asset accounts, item / category / location rules, and resolving an asset's accounts.

use crate::installed;
use async_trait::async_trait;
use ferrum_domain::util::{b, find, get, id, rows, s, set, sys};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, FieldError, Filter, HookCtx, Kernel, Result, Tx};
use serde_json::{Value, json};
use std::collections::HashSet;
use uuid::Uuid;

/// The standard asset accounts: (company field, name, number, parent group, root type, account type).
const ACCOUNTS: &[(&str, &str, &str, &str, &str, &str)] = &[
    ("fa_capital_work_in_progress_account", "Capital Work in Progress", "1240", "Fixed Assets", "Asset", "Capital Work in Progress"),
    ("fa_accumulated_depreciation_account", "Accumulated Depreciation", "1250", "Fixed Assets", "Asset", "Accumulated Depreciation"),
    ("fa_depreciation_expense_account", "Depreciation", "5290", "Indirect Expenses", "Expense", "Depreciation"),
    ("fa_disposal_account", "Gain/Loss on Asset Disposal", "5295", "Indirect Expenses", "Expense", ""),
];

/// Creates (or adopts by name) the standard asset accounts the company has no default for yet and
/// links them; the depreciation cost center defaults to the company's. Idempotent.
pub(crate) async fn ensure_accounts(k: &Kernel, ctx: &Ctx, tx: &mut Tx, company: Uuid) -> Result<usize> {
    let sctx = sys(ctx);
    let co = get(k, ctx, tx, "Company", company).await?;
    let mut linked = Doc::new();
    for (field, name, number, parent, root, typ) in ACCOUNTS {
        if id(&co, field).is_some() {
            continue;
        }
        let by = |f: &str, v: &str| vec![Filter::eq("company", company.to_string()), Filter::eq(f, v)];
        let existing = find(k, ctx, tx, "Account", by("account_name", name), None, 1).await?;
        let account = match existing.first() {
            // An account of that name that cannot take postings is left for the user to sort out.
            Some(a) if b(a, "is_group") || s(a, "root_type") != Some(*root) => continue,
            Some(a) => id(a, "id"),
            None => {
                let group = find(k, ctx, tx, "Account", by("account_name", parent), None, 1).await?;
                let taken = !find(k, ctx, tx, "Account", by("account_number", number), None, 1).await?.is_empty();
                let mut a = Doc::new();
                set(&mut a, "account_name", *name);
                set(&mut a, "company", company.to_string());
                set(&mut a, "root_type", *root);
                set(&mut a, "account_currency", co.get("default_currency").cloned().unwrap_or(Value::Null));
                if !typ.is_empty() {
                    set(&mut a, "account_type", *typ);
                }
                if !taken {
                    set(&mut a, "account_number", *number);
                }
                if let Some(g) = group.first().filter(|g| b(g, "is_group") && s(g, "root_type") == Some(*root)) {
                    set(&mut a, "parent_account", g.get("id").cloned().unwrap_or(Value::Null));
                }
                id(&k.insert_tx(&sctx, tx, "Account", a).await?, "id")
            }
        };
        if let Some(a) = account {
            linked.insert(field.to_string(), json!(a.to_string()));
        }
    }
    if id(&co, "fa_depreciation_cost_center").is_none()
        && let Some(cc) = id(&co, "default_cost_center")
    {
        linked.insert("fa_depreciation_cost_center".into(), json!(cc.to_string()));
    }
    let n = linked.len();
    if n > 0 {
        k.set_values_tx(&sctx, tx, "Company", company, linked).await?;
    }
    Ok(n)
}

/// An asset category's accounts for one company, with the company's defaults filled in.
#[derive(Debug, Clone)]
pub(crate) struct Accounts {
    pub category: String,
    pub fixed_asset: Uuid,
    pub accumulated: Option<Uuid>,
    pub expense: Option<Uuid>,
    pub cwip: Option<Uuid>,
    pub disposal: Option<Uuid>,
}

impl Accounts {
    fn need(&self, v: Option<Uuid>, what: &str) -> Result<Uuid> {
        v.ok_or_else(|| {
            Error::invalid(format!(
                "set the {what} account of asset category {} for this company, or the company's default (Company → Create Asset Accounts)",
                self.category
            ))
        })
    }
    pub fn accumulated(&self) -> Result<Uuid> {
        self.need(self.accumulated, "accumulated depreciation")
    }
    pub fn expense(&self) -> Result<Uuid> {
        self.need(self.expense, "depreciation expense")
    }
    pub fn cwip(&self) -> Result<Uuid> {
        self.need(self.cwip, "capital work in progress")
    }
    pub fn disposal(&self) -> Result<Uuid> {
        self.need(self.disposal, "gain/loss on disposal")
    }
}

/// The category's accounts for `company`, falling back to the company's defaults (the standard
/// asset accounts are created the first time a default is missing). Returns the category too.
pub(crate) async fn accounts(k: &Kernel, ctx: &Ctx, tx: &mut Tx, category: Uuid, company: Uuid) -> Result<(Doc, Accounts)> {
    let cat = get(k, ctx, tx, "AssetCategory", category).await?;
    let name = s(&cat, "category_name").unwrap_or_default().to_string();
    let row = rows(&cat, "accounts")
        .into_iter()
        .find(|r| id(r, "company") == Some(company))
        .ok_or_else(|| Error::invalid(format!("asset category {name} has no accounts for this company (Asset Category → Accounts per Company)")))?;
    let fixed_asset = id(&row, "fixed_asset_account").ok_or_else(|| Error::invalid(format!("asset category {name} has no fixed asset account")))?;
    let mut co = get(k, ctx, tx, "Company", company).await?;
    let pairs = [
        ("accumulated_depreciation_account", "fa_accumulated_depreciation_account"),
        ("depreciation_expense_account", "fa_depreciation_expense_account"),
        ("capital_work_in_progress_account", "fa_capital_work_in_progress_account"),
        ("disposal_account", "fa_disposal_account"),
    ];
    if pairs.iter().any(|(own, default)| id(&row, own).is_none() && id(&co, default).is_none()) && ensure_accounts(k, ctx, tx, company).await? > 0 {
        co = get(k, ctx, tx, "Company", company).await?;
    }
    let pick = |own: &str, default: &str| id(&row, own).or_else(|| id(&co, default));
    let accounts = Accounts {
        category: name,
        fixed_asset,
        accumulated: pick(pairs[0].0, pairs[0].1),
        expense: pick(pairs[1].0, pairs[1].1),
        cwip: pick(pairs[2].0, pairs[2].1),
        disposal: pick(pairs[3].0, pairs[3].1),
    };
    Ok((cat, accounts))
}

/// Refuses an account of another company or one that is a group.
pub(crate) async fn check_account(k: &Kernel, ctx: &Ctx, tx: &mut Tx, account: Uuid, company: Uuid, field: &str) -> Result<Doc> {
    let a = get(k, ctx, tx, "Account", account).await?;
    if id(&a, "company") != Some(company) {
        return Err(Error::field(field, "belongs to another company"));
    }
    if b(&a, "is_group") {
        return Err(Error::field(field, "is a group account"));
    }
    Ok(a)
}

pub struct CompanyHook;

#[async_trait]
impl DocHook for CompanyHook {
    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        // The chart of accounts exists by now: the platform's company hook runs first.
        if old.is_none() && installed(hc.ctx) {
            let company = id(doc, "id").ok_or_else(|| Error::internal("company without id"))?;
            ensure_accounts(hc.kernel, hc.ctx, hc.tx, company).await?;
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        if name != "setup_asset_accounts" || !installed(hc.ctx) {
            return Ok(None);
        }
        let company = id(doc, "id").ok_or_else(|| Error::internal("company without id"))?;
        let linked = ensure_accounts(hc.kernel, hc.ctx, hc.tx, company).await?;
        Ok(Some(json!({"linked": linked, "message": if linked > 0 { "Asset accounts set up" } else { "This company's asset accounts are already set" }})))
    }
}

pub struct ItemHook;

#[async_trait]
impl DocHook for ItemHook {
    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if installed(hc.ctx) && b(doc, "fa_is_fixed_asset") && b(doc, "is_stock_item") {
            return Err(Error::field("is_stock_item", "a fixed asset item cannot be a stock item"));
        }
        Ok(())
    }
}

pub struct CategoryHook;

#[async_trait]
impl DocHook for CategoryHook {
    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let mut errors = Vec::new();
        let mut seen = HashSet::new();
        for (i, r) in rows(doc, "accounts").iter().enumerate() {
            let Some(company) = id(r, "company") else { continue };
            if !seen.insert(company) {
                errors.push(FieldError::new("company", "this company already has a row").in_row("accounts", i));
                continue;
            }
            for f in [
                "fixed_asset_account",
                "accumulated_depreciation_account",
                "depreciation_expense_account",
                "capital_work_in_progress_account",
                "disposal_account",
            ] {
                let Some(acc) = id(r, f) else { continue };
                let a = get(hc.kernel, hc.ctx, hc.tx, "Account", acc).await?;
                if id(&a, "company") != Some(company) {
                    errors.push(FieldError::new(f, "belongs to another company").in_row("accounts", i));
                } else if f == "disposal_account" && !matches!(s(&a, "root_type"), Some("Income" | "Expense")) {
                    errors.push(FieldError::new(f, "must be an income or expense account").in_row("accounts", i));
                }
            }
            if id(r, "fixed_asset_account").is_some() && id(r, "fixed_asset_account") == id(r, "accumulated_depreciation_account") {
                errors.push(FieldError::new("accumulated_depreciation_account", "must differ from the fixed asset account").in_row("accounts", i));
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) }
    }
}

pub struct LocationHook;

#[async_trait]
impl DocHook for LocationHook {
    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let Some(loc) = id(doc, "id") else { return Ok(()) };
        let (holds_assets, has_children): (bool, bool) = sqlx::query_as(
            "select exists(select 1 from e_asset where tenant_id = $1 and location = $2 and docstatus < 2), \
             exists(select 1 from e_location where tenant_id = $1 and parent_location = $2)",
        )
        .bind(hc.ctx.tenant)
        .bind(loc)
        .fetch_one(&mut **hc.tx)
        .await?;
        if b(doc, "is_group") && holds_assets {
            return Err(Error::field("is_group", "assets are at this location; a group cannot hold assets"));
        }
        if !b(doc, "is_group") && has_children {
            return Err(Error::field("is_group", "other locations are inside this one; it must stay a group"));
        }
        Ok(())
    }
}
