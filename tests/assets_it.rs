//! End-to-end assets test against a real PostgreSQL (`FERRUM_ASSETS_TEST_DATABASE_URL`, default
//! `postgres://ferrum:ferrum@localhost:5432/ferrum_assets_test`). The database is reset first.
//!
//! A company buys laptops, chairs and a boiler on one purchase invoice (CWIP for the boiler) and
//! registers two assets it already owned (one with opening depreciation). Depreciation is booked
//! by the periodic upkeep (twice, to prove it idempotent, and across a closed period to prove it
//! resumable), a laptop is impaired mid-life, another gets a capitalized repair and moves between
//! custodians, the boiler is scrapped and restored, the chairs are sold at a loss, the sale undone
//! and redone at a gain, a fully depreciated laptop is sold. Every figure is checked to the cent,
//! the register ties to the ledger, and every refusal path is exercised. All dates are in the
//! past, so the expected figures hold whatever day the test runs.

use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings, ListQuery};
use ferrum_meta::Catalog;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn dec(v: &Value) -> Decimal {
    match v {
        Value::String(s) => Decimal::from_str(s).unwrap_or_else(|_| panic!("not a decimal: {s}")),
        Value::Number(n) => Decimal::from_str(&n.to_string()).unwrap(),
        Value::Null => Decimal::ZERO,
        other => panic!("not a decimal: {other}"),
    }
}

fn n(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn uid(doc: &Doc) -> Uuid {
    doc["id"].as_str().unwrap().parse().unwrap()
}

fn rows(doc: &Doc, k: &str) -> Vec<Doc> {
    doc[k].as_array().unwrap().iter().map(|r| r.as_object().unwrap().clone()).collect()
}

/// `(date, amount, booked)` of an asset's schedule.
fn schedule(doc: &Doc) -> Vec<(String, Decimal, bool)> {
    rows(doc, "schedule")
        .iter()
        .map(|r| (r["schedule_date"].as_str().unwrap().to_string(), dec(&r["depreciation_amount"]), r["booked"] == json!(true)))
        .collect()
}

fn amounts(doc: &Doc) -> Vec<Decimal> {
    schedule(doc).into_iter().map(|r| r.1).collect()
}

fn validation(e: &Error, needle: &str) -> bool {
    match e {
        Error::Validation(f) => f.iter().any(|x| x.message.contains(needle) || x.field == needle),
        Error::Invalid(m) | Error::Conflict(m) => m.contains(needle),
        _ => false,
    }
}

struct T {
    k: Arc<Kernel>,
    tenant: Uuid,
    admin: Ctx,
}

impl T {
    async fn insert(&self, ctx: &Ctx, entity: &str, v: Value) -> Doc {
        self.k.insert(ctx, entity, obj(v)).await.unwrap_or_else(|e| panic!("insert {entity}: {e:?}"))
    }
    async fn submit(&self, ctx: &Ctx, entity: &str, id: Uuid) -> Doc {
        self.k.submit(ctx, entity, id, None).await.unwrap_or_else(|e| panic!("submit {entity}: {e:?}"))
    }
    async fn cancel(&self, ctx: &Ctx, entity: &str, id: Uuid) -> Doc {
        self.k.cancel(ctx, entity, id, None).await.unwrap_or_else(|e| panic!("cancel {entity}: {e:?}"))
    }
    async fn update(&self, ctx: &Ctx, entity: &str, id: Uuid, v: Value) -> Doc {
        self.k.update(ctx, entity, id, obj(v), None).await.unwrap_or_else(|e| panic!("update {entity}: {e:?}"))
    }
    async fn get(&self, entity: &str, id: Uuid) -> Doc {
        self.k.get(&self.admin, entity, id, false).await.unwrap_or_else(|e| panic!("get {entity}: {e:?}"))
    }
    async fn list(&self, ctx: &Ctx, entity: &str, filters: Value) -> Vec<Doc> {
        let q = ListQuery { filters: ferrum_kernel::query::parse_filters(&filters).unwrap(), limit: Some(500), ..Default::default() };
        self.k.list(ctx, entity, &q).await.unwrap_or_else(|e| panic!("list {entity}: {e:?}")).rows
    }
    async fn find_one(&self, entity: &str, filters: Value) -> Doc {
        let found = self.list(&self.admin, entity, filters.clone()).await;
        assert_eq!(found.len(), 1, "{entity} {filters}");
        found[0].clone()
    }
    async fn count(&self, sql: &str, id: Option<Uuid>) -> i64 {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string())).bind(self.tenant).bind(id).fetch_one(&mut *tx).await.unwrap()
    }
    /// Debit minus credit on an account (by name) across all postings.
    async fn gl(&self, account: &str) -> Decimal {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        sqlx::query_scalar(
            "select coalesce(sum(l.debit - l.credit), 0) from journal_line l join e_account a on a.tenant_id = l.tenant_id and a.id = l.account_id \
             where l.tenant_id = $1 and a.account_name = $2",
        )
        .bind(self.tenant)
        .bind(account)
        .fetch_one(&mut *tx)
        .await
        .unwrap()
    }
    /// Debit minus credit an asset's own entries (by source document) left on an account.
    async fn gl_for(&self, account: &str, sources: &[Uuid]) -> Decimal {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        sqlx::query_scalar(
            "select coalesce(sum(l.debit - l.credit), 0) from journal_line l join journal_entry e on e.tenant_id = l.tenant_id and e.id = l.entry_id \
             join e_account a on a.tenant_id = l.tenant_id and a.id = l.account_id where l.tenant_id = $1 and a.account_name = $2 and e.source_id = any($3)",
        )
        .bind(self.tenant)
        .bind(account)
        .bind(sources)
        .fetch_one(&mut *tx)
        .await
        .unwrap()
    }
    async fn depreciation_entries(&self) -> i64 {
        self.count("select count(*) from e_journal_entry where tenant_id = $1 and entry_type = 'Depreciation' and docstatus = 1 and $2::uuid is null", None)
            .await
    }
    async fn report(&self, ctx: &Ctx, name: &str, params: &[(&str, String)]) -> Value {
        let p: HashMap<String, String> = params.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        ferrum_domain::reports::run(&self.k, ctx, name, &p).await.unwrap_or_else(|e| panic!("report {name}: {e:?}"))
    }
    async fn user(&self, email: &str, role: &str) -> Ctx {
        let id = self.k.create_user(&self.admin, email, email, Some("Correct-Horse-9"), &[role.into()], false).await.expect("user");
        self.k.load_ctx(self.tenant, id, None, "it".into()).await.expect("ctx")
    }
    async fn upkeep(&self) {
        ferrum_app_assets::maintenance(&self.k).await.expect("asset upkeep");
    }
}

#[test]
fn assets_end_to_end() {
    // Same runtime shape as the server: unoptimized async hook chains need more than 2 MiB.
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_ASSETS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_assets_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 10, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_ops::packages().expect("ops"));
    packages.extend(ferrum_app_erp::packages().expect("ERP's packages"));
    packages.extend(ferrum_app_assets::packages().expect("app packages"));
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_erp::register(&mut hooks);
    for source in ferrum_app_erp::reports() {
        ferrum_domain::reports::add_source(source);
    }
    ferrum_app_assets::register(&mut hooks);
    for source in ferrum_app_assets::reports() {
        ferrum_domain::reports::add_source(source);
    }
    let k = Kernel::boot(db, catalog, hooks, KernelSettings::default(), None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.expect("partitions");
    ferrum_app_erp::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "acme".into(),
            name: "Acme Industries".into(),
            admin_email: "admin@acme.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["assets".into()],
            settings: Some(json!({"fiscal_year_start_month": 4})),
        })
        .await
        .expect("tenant");
    let session = k.login("acme", "admin@acme.test", "Correct-Horse-9").await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let t = T { k: k.clone(), tenant, admin };
    let a = t.admin.clone();

    // ───────────── setup: company (its asset accounts come with it), locations, categories ─────────────
    let inr = t.find_one("Currency", json!({"code": "INR"})).await;
    let company = t.insert(&a, "Company", json!({"company_name": "Acme Industries", "abbr": "AI", "default_currency": inr["id"]})).await;
    let cid = uid(&company);
    let co = t.get("Company", cid).await;
    let account = |name: &'static str| {
        let t = &t;
        async move { uid(&t.find_one("Account", json!({"account_name": name, "company": cid.to_string()})).await) }
    };
    for (field, name) in [
        ("fa_accumulated_depreciation_account", "Accumulated Depreciation"),
        ("fa_depreciation_expense_account", "Depreciation"),
        ("fa_capital_work_in_progress_account", "Capital Work in Progress"),
        ("fa_disposal_account", "Gain/Loss on Asset Disposal"),
    ] {
        assert_eq!(co[field], json!(account(name).await.to_string()), "{field} is set up with the company");
    }
    assert_eq!(co["fa_depreciation_cost_center"], co["default_cost_center"]);
    let again = t.k.run_action(&a, "Company", cid, "setup_asset_accounts", json!({})).await.unwrap();
    assert_eq!(again["linked"], json!(0), "setting up twice changes nothing");

    let fixed_assets = account("Fixed Assets").await;
    let vehicles = uid(&t
        .insert(&a, "Account", json!({"account_name": "Vehicles", "company": cid.to_string(), "parent_account": fixed_assets.to_string(), "root_type": "Asset", "account_type": "Fixed Asset"}))
        .await);
    let (computers, machinery, furniture) = (account("Computers").await, account("Plant and Machinery").await, account("Furniture and Fixtures").await);
    let admin_expenses = account("Administrative Expenses").await;
    let opening_equity = account("Opening Balance Equity").await;

    let head_office = uid(&t.insert(&a, "Location", json!({"location_name": "Head Office", "is_group": true})).await);
    let floor1 = uid(&t.insert(&a, "Location", json!({"location_name": "HQ Floor 1", "parent_location": head_office.to_string()})).await);
    let floor2 = uid(&t.insert(&a, "Location", json!({"location_name": "HQ Floor 2", "parent_location": head_office.to_string()})).await);
    let plant = uid(&t.insert(&a, "Location", json!({"location_name": "Plant"})).await);
    let cycle = t.k.update(&a, "Location", head_office, obj(json!({"parent_location": head_office.to_string()})), None).await;
    assert!(cycle.is_err(), "a location cannot contain itself");

    let category = |name: &str, extra: Value, acc: Uuid| {
        let mut v = json!({"category_name": name, "accounts": [{"company": cid.to_string(), "fixed_asset_account": acc.to_string()}]});
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        v
    };
    let laptops = uid(&t
        .insert(
            &a,
            "AssetCategory",
            category(
                "Laptops",
                json!({"depreciation_method": "Straight Line", "total_number_of_depreciations": 36, "frequency_of_depreciation": 1}),
                computers,
            ),
        )
        .await);
    let presses = uid(&t
        .insert(
            &a,
            "AssetCategory",
            category(
                "Machinery",
                json!({"depreciation_method": "Written Down Value", "rate_of_depreciation": "40", "total_number_of_depreciations": 5}),
                machinery,
            ),
        )
        .await);
    let chairs_cat =
        uid(&t.insert(&a, "AssetCategory", category("Furniture", json!({"total_number_of_depreciations": 5, "group_assets": true}), furniture)).await);
    let boilers =
        uid(&t.insert(&a, "AssetCategory", category("Plant", json!({"total_number_of_depreciations": 10, "enable_cwip_accounting": true}), machinery)).await);
    let vans = uid(&t.insert(&a, "AssetCategory", category("Vehicles", json!({"total_number_of_depreciations": 5}), vehicles)).await);
    let wdv_without_rate =
        t.k.insert(&a, "AssetCategory", obj(json!({"category_name": "Bad", "depreciation_method": "Written Down Value", "total_number_of_depreciations": 3})))
            .await;
    assert!(wdv_without_rate.is_err(), "written down value needs a rate");

    let products = t.find_one("ItemGroup", json!({"group_name": "Products"})).await;
    let nos = t.find_one("Uom", json!({"uom_name": "Nos"})).await;
    let item = |code: &str, cat: Uuid| {
        json!({"item_code": code, "item_name": code.to_lowercase(), "item_group": products["id"], "stock_uom": nos["id"], "is_stock_item": false,
               "fa_is_fixed_asset": true, "fa_asset_category": cat.to_string()})
    };
    let laptop_item = uid(&t.insert(&a, "Item", item("LAPTOP", laptops)).await);
    let press_item = uid(&t.insert(&a, "Item", item("PRESS", presses)).await);
    let chair_item = uid(&t.insert(&a, "Item", item("CHAIR", chairs_cat)).await);
    let boiler_item = uid(&t.insert(&a, "Item", item("BOILER", boilers)).await);
    let van_item = uid(&t.insert(&a, "Item", item("VAN", vans)).await);
    let stocked =
        t.k.insert(
            &a,
            "Item",
            obj(json!({"item_code": "BAD", "item_name": "bad", "item_group": products["id"], "stock_uom": nos["id"], "fa_is_fixed_asset": true})),
        )
        .await;
    assert!(matches!(stocked, Err(ref e) if validation(e, "is_stock_item")), "a fixed asset item is never a stock item: {stocked:?}");
    let pencil = uid(&t
        .insert(&a, "Item", json!({"item_code": "PENCIL", "item_name": "pencil", "item_group": products["id"], "stock_uom": nos["id"], "is_stock_item": false}))
        .await);

    let manager = t.user("manager@acme.test", "Asset Manager").await;
    let alice = t.user("alice@acme.test", "Asset User").await;
    let bob = t.user("bob@acme.test", "Asset User").await;
    let accountant = t.user("accountant@acme.test", "Accounts Manager").await;
    let supplier = uid(&t.insert(&a, "Supplier", json!({"supplier_name": "Dell"})).await);
    let customer = uid(&t.insert(&a, "Customer", json!({"customer_name": "Scrap & Resale Co"})).await);

    // The three assets bought before Ferrum are in the opening balances.
    let opening = t
        .insert(
            &a,
            "JournalEntry",
            json!({"company": cid.to_string(), "posting_date": "2019-01-01", "entry_type": "Opening Entry", "accounts": [
                {"account": machinery.to_string(), "debit": "11000"},
                {"account": vehicles.to_string(), "debit": "10000"},
                {"account": account("Accumulated Depreciation").await.to_string(), "credit": "4000"},
                {"account": opening_equity.to_string(), "credit": "17000"}]}),
        )
        .await;
    t.submit(&a, "JournalEntry", uid(&opening)).await;

    // ───────────── capitalization from a purchase invoice ─────────────
    let pi = t
        .insert(
            &a,
            "PurchaseInvoice",
            json!({"company": cid.to_string(), "supplier": supplier.to_string(), "bill_no": "DELL-1", "posting_date": "2023-01-15", "items": [
                {"item": laptop_item.to_string(), "qty": "3", "price_list_rate": "1200", "fa_asset_location": floor1.to_string()},
                {"item": chair_item.to_string(), "qty": "10", "price_list_rate": "150", "fa_asset_location": floor2.to_string()},
                {"item": boiler_item.to_string(), "qty": "1", "price_list_rate": "50000", "fa_asset_location": plant.to_string()},
                {"item": pencil.to_string(), "qty": "5", "price_list_rate": "2"}]}),
        )
        .await;
    let lines = rows(&pi, "items");
    assert_eq!(lines[0]["fa_is_fixed_asset"], json!(true), "marked from the item");
    assert_eq!(lines[0]["fa_asset_category"], json!(laptops.to_string()));
    assert_eq!(lines[0]["expense_account"], json!(computers.to_string()), "booked to the category's fixed asset account");
    assert_eq!(lines[2]["expense_account"], json!(account("Capital Work in Progress").await.to_string()), "CWIP for a category that uses it");
    assert_ne!(lines[3]["fa_is_fixed_asset"], json!(true));
    let no_location = t
        .k
        .insert(
            &a,
            "PurchaseInvoice",
            obj(json!({"company": cid.to_string(), "supplier": supplier.to_string(), "bill_no": "DELL-X", "items": [{"item": laptop_item.to_string(), "qty": "1", "price_list_rate": "1"}]})),
        )
        .await;
    assert!(matches!(no_location, Err(ref e) if validation(e, "fa_asset_location")), "an asset line needs its location: {no_location:?}");
    let half =
        t.k.insert(
            &a,
            "PurchaseInvoice",
            obj(json!({"company": cid.to_string(), "supplier": supplier.to_string(), "bill_no": "DELL-Y",
                       "items": [{"item": laptop_item.to_string(), "qty": "1.5", "price_list_rate": "1", "fa_asset_location": floor1.to_string()}]})),
        )
        .await;
    assert!(matches!(half, Err(ref e) if validation(e, "whole units")), "{half:?}");
    let pi_id = uid(&pi);
    t.submit(&a, "PurchaseInvoice", pi_id).await;
    assert_eq!(t.gl("Computers").await, n("3600"));
    assert_eq!(t.gl("Furniture and Fixtures").await, n("1500"));
    assert_eq!(t.gl("Capital Work in Progress").await, n("50000"));
    assert_eq!(t.gl("Creditors").await, n("-55110"));

    let drafts = t.list(&a, "Asset", json!({"purchase_invoice": pi_id.to_string()})).await;
    assert_eq!(drafts.len(), 5, "three laptops, one group of chairs, one boiler");
    let of_item = |it: Uuid| drafts.iter().filter(|d| d["item"] == json!(it.to_string())).map(uid).collect::<Vec<_>>();
    let laptop_ids = of_item(laptop_item);
    let (chairs, boiler) = (of_item(chair_item)[0], of_item(boiler_item)[0]);
    assert_eq!(laptop_ids.len(), 3);
    let (laptop1, laptop2, laptop3) = (laptop_ids[0], laptop_ids[1], laptop_ids[2]);
    let l1 = t.get("Asset", laptop1).await;
    assert_eq!((l1["status"].clone(), dec(&l1["gross_purchase_amount"]), l1["asset_name"].clone()), (json!("Draft"), n("1200"), json!("laptop")));
    assert_eq!(l1["location"], json!(floor1.to_string()));
    assert_eq!((l1["available_for_use_date"].clone(), l1["depreciation_start_date"].clone()), (json!("2023-01-15"), json!("2023-01-31")));
    // Straight line, 36 months, in use from Jan 15: 17/31 of January, the other 14/31 at the end.
    let s1 = schedule(&l1);
    assert_eq!(s1.len(), 37, "a draft shows its schedule");
    assert_eq!((s1[0].0.as_str(), s1[0].1), ("2023-01-31", n("18.28")));
    assert!(s1[1..36].iter().all(|r| r.1 == n("33.33")));
    assert_eq!((s1[36].0.as_str(), s1[36].1), ("2026-01-31", n("15.17")), "the last row absorbs the rounding");
    assert_eq!(amounts(&l1).iter().sum::<Decimal>(), n("1200"));
    let ch = t.get("Asset", chairs).await;
    assert_eq!((ch["asset_quantity"].clone(), dec(&ch["gross_purchase_amount"])), (json!(10), n("1500")), "one grouped asset for the line");
    let edit = t.k.update(&a, "Asset", laptop1, obj(json!({"gross_purchase_amount": "1000"})), None).await;
    assert!(matches!(edit, Err(ref e) if validation(e, "purchase invoice")), "the invoice fixes the cost: {edit:?}");
    let delete = t.k.delete(&a, "Asset", laptop1, None).await;
    assert!(matches!(delete, Err(Error::Invalid(ref m)) if m.contains("purchase invoice")), "{delete:?}");

    // ───────────── assets already in the books ─────────────
    let press = t
        .insert(
            &a,
            "Asset",
            json!({"item": press_item.to_string(), "company": cid.to_string(), "location": plant.to_string(), "is_existing_asset": true,
                   "purchase_date": "2020-04-01", "gross_purchase_amount": "10000", "salvage_value": "1000"}),
        )
        .await;
    assert_eq!(press["depreciation_method"], json!("Written Down Value"), "settings from the category");
    assert_eq!(press["depreciation_start_date"], json!("2021-03-31"), "the end of the first year");
    assert_eq!(amounts(&press), [n("4000"), n("2400"), n("1440"), n("864"), n("296")], "40% of the value left; the last row ends at salvage");
    let press = uid(&press);
    let van = t
        .insert(
            &a,
            "Asset",
            json!({"item": van_item.to_string(), "company": cid.to_string(), "location": plant.to_string(), "is_existing_asset": true,
                   "purchase_date": "2019-01-01", "gross_purchase_amount": "10000", "salvage_value": "1000",
                   "opening_accumulated_depreciation": "4000", "opening_number_of_booked_depreciations": 2}),
        )
        .await;
    assert_eq!(van["depreciation_start_date"], json!("2021-12-31"), "the first period still to come");
    assert_eq!(
        schedule(&van),
        [("2021-12-31".into(), n("1666.67"), false), ("2022-12-31".into(), n("1666.67"), false), ("2023-12-31".into(), n("1666.66"), false)]
    );
    let van_accumulated: Vec<Decimal> = rows(&van, "schedule").iter().map(|r| dec(&r["accumulated_depreciation"])).collect();
    assert_eq!(van_accumulated, [n("5666.67"), n("7333.34"), n("9000")], "opening depreciation counts");
    let van = uid(&van);
    // A manual schedule: the user's own rows, checked against what is depreciable.
    let manual = |second: &str| {
        json!({"item": press_item.to_string(), "asset_name": "Old lathe", "company": cid.to_string(), "location": plant.to_string(), "is_existing_asset": true,
               "purchase_date": "2022-01-01", "gross_purchase_amount": "1000", "depreciation_method": "Manual", "schedule": [
                   {"schedule_date": "2022-12-31", "depreciation_amount": "300"}, {"schedule_date": "2023-12-31", "depreciation_amount": second},
                   {"schedule_date": "2024-12-31", "depreciation_amount": "200"}]})
    };
    let too_much = t.k.insert(&a, "Asset", obj(manual("600"))).await;
    assert!(matches!(too_much, Err(ref e) if validation(e, "only 1000 is depreciable")), "{too_much:?}");
    let lathe = t.insert(&a, "Asset", manual("300")).await;
    let lathe_accumulated: Vec<Decimal> = rows(&lathe, "schedule").iter().map(|r| dec(&r["accumulated_depreciation"])).collect();
    assert_eq!(lathe_accumulated, [n("300"), n("600"), n("800")], "a partial plan: 200 stays on the books");
    let lathe = uid(&lathe);
    let not_bought = t
        .insert(&a, "Asset", json!({"item": laptop_item.to_string(), "company": cid.to_string(), "location": floor1.to_string(), "purchase_date": "2024-01-01", "gross_purchase_amount": "500"}))
        .await;
    let refused = t.k.submit(&a, "Asset", uid(&not_bought), None).await;
    assert!(matches!(refused, Err(ref e) if validation(e, "purchase_invoice")), "a new asset comes from its invoice: {refused:?}");
    t.k.delete(&a, "Asset", uid(&not_bought), None).await.unwrap();
    let wrong_item =
        t.k.insert(
            &a,
            "Asset",
            obj(json!({"item": pencil.to_string(), "company": cid.to_string(), "location": floor1.to_string(), "is_existing_asset": true,
                                          "asset_category": laptops.to_string(), "purchase_date": "2024-01-01", "gross_purchase_amount": "5"})),
        )
        .await;
    assert!(matches!(wrong_item, Err(ref e) if validation(e, "not a fixed asset item")), "{wrong_item:?}");
    let group_location =
        t.k.insert(
            &a,
            "Asset",
            obj(json!({"item": van_item.to_string(), "company": cid.to_string(), "location": head_office.to_string(), "is_existing_asset": true,
                                            "purchase_date": "2024-01-01", "gross_purchase_amount": "5"})),
        )
        .await;
    assert!(group_location.is_err(), "assets are at a location, not a group");

    t.update(&a, "Asset", laptop1, json!({"custodian": "Alice@acme.test"})).await;
    for asset in [laptop1, press, van] {
        let done = t.submit(&manager, "Asset", asset).await;
        assert!(!done["fixed_asset_account"].is_null(), "accounts fixed on submit");
    }
    assert_eq!(t.get("Asset", van).await["status"], json!("Partially Depreciated"), "opening depreciation");
    assert_eq!(t.get("Asset", laptop1).await["status"], json!("Submitted"));

    // ───────────── the upkeep books what is due, once ─────────────
    t.upkeep().await;
    assert_eq!(t.depreciation_entries().await, 37 + 5 + 3);
    let l1 = t.get("Asset", laptop1).await;
    assert!(schedule(&l1).iter().all(|r| r.2), "every laptop row is due");
    assert_eq!(
        (dec(&l1["accumulated_depreciation"]), dec(&l1["value_after_depreciation"]), l1["status"].clone()),
        (n("1200"), n("0"), json!("Fully Depreciated"))
    );
    let first = &rows(&l1, "schedule")[0];
    let je = t.get("JournalEntry", first["journal_entry"].as_str().unwrap().parse().unwrap()).await;
    assert_eq!(
        (je["posting_date"].clone(), je["entry_type"].clone(), je["fa_asset"].clone()),
        (json!("2023-01-31"), json!("Depreciation"), json!(laptop1.to_string()))
    );
    assert_eq!(rows(&je, "accounts")[0]["cost_center"], co["default_cost_center"], "on the asset's cost center");
    let pr = t.get("Asset", press).await;
    assert_eq!((dec(&pr["value_after_depreciation"]), pr["status"].clone()), (n("1000"), json!("Fully Depreciated")), "down to the salvage value, not below");
    let vn = t.get("Asset", van).await;
    assert_eq!((dec(&vn["accumulated_depreciation"]), dec(&vn["value_after_depreciation"])), (n("9000"), n("1000")));
    t.upkeep().await;
    assert_eq!(t.depreciation_entries().await, 45, "a second run books nothing again");
    assert_eq!(t.gl("Depreciation").await, n("1200") + n("9000") + n("5000"));
    let cancel_je = t.k.cancel(&a, "JournalEntry", uid(&je), None).await;
    assert!(matches!(cancel_je, Err(Error::Invalid(ref m)) if m.contains("books depreciation")), "{cancel_je:?}");

    // ───────────── the rest of the purchase goes into service ─────────────
    t.update(&a, "Asset", laptop3, json!({"custodian": "bob@acme.test"})).await;
    for asset in [laptop2, laptop3, chairs, boiler, lathe] {
        t.submit(&manager, "Asset", asset).await;
    }
    assert_eq!(t.gl("Capital Work in Progress").await, Decimal::ZERO, "the boiler left CWIP when it was submitted");
    assert_eq!(t.gl("Plant and Machinery").await, n("61000"));
    let refused = t.k.cancel(&a, "PurchaseInvoice", pi_id, None).await;
    assert!(matches!(refused, Err(Error::Invalid(ref m)) if m.contains("cancel them first")), "{refused:?}");

    // ───────────── impairment mid-life, books the depreciation due first ─────────────
    let adjust = |value: &'static str| json!({"asset": laptop2.to_string(), "adjustment_date": "2024-06-30", "new_asset_value": value, "difference_account": admin_expenses.to_string()});
    let below =
        t.k.insert(
            &manager,
            "AssetValueAdjustment",
            obj(json!({"asset": van.to_string(), "adjustment_date": "2024-06-30", "new_asset_value": "500", "difference_account": admin_expenses.to_string()})),
        )
        .await;
    assert!(matches!(below, Err(ref e) if validation(e, "salvage")), "{below:?}");
    let early =
        t.k.insert(
            &manager,
            "AssetValueAdjustment",
            obj(
                json!({"asset": van.to_string(), "adjustment_date": "2023-06-30", "new_asset_value": "1000", "difference_account": admin_expenses.to_string()}),
            ),
        )
        .await;
    assert!(matches!(early, Err(ref e) if validation(e, "booked up to")), "{early:?}");
    let a1 = t.insert(&manager, "AssetValueAdjustment", adjust("400")).await;
    assert_eq!(dec(&a1["current_asset_value"]), n("1200"), "a preview from what is booked");
    let a1 = t.submit(&manager, "AssetValueAdjustment", uid(&a1)).await;
    // 18 months booked first: 18.28 + 17 × 33.33.
    assert_eq!((dec(&a1["current_asset_value"]), dec(&a1["difference_amount"])), (n("615.11"), n("-215.11")));
    let l2 = t.get("Asset", laptop2).await;
    let s2 = schedule(&l2);
    assert_eq!(s2.iter().filter(|r| r.2).count(), 18);
    let rest: Vec<Decimal> = s2.iter().filter(|r| !r.2).map(|r| r.1).collect();
    assert_eq!(rest.len(), 19);
    assert!(rest[..18].iter().all(|x| *x == n("21.68")) && rest[18] == n("9.76"), "400 spread over the rest of its life: {rest:?}");
    assert_eq!((dec(&l2["total_asset_cost"]), dec(&l2["value_after_depreciation"])), (n("984.89"), n("400")));
    assert_eq!(t.gl_for("Computers", &[uid(&a1)]).await, n("-215.11"));
    assert_eq!(t.gl_for("Administrative Expenses", &[uid(&a1)]).await, n("215.11"));
    t.cancel(&manager, "AssetValueAdjustment", uid(&a1)).await;
    let l2 = t.get("Asset", laptop2).await;
    let rest: Vec<Decimal> = schedule(&l2).iter().filter(|r| !r.2).map(|r| r.1).collect();
    assert_eq!(rest.iter().sum::<Decimal>(), n("615.11"), "back to the old value");
    assert!(rest[..18].iter().all(|x| *x == n("33.34")) && rest[18] == n("14.99"), "{rest:?}");
    assert_eq!(t.gl_for("Computers", &[uid(&a1)]).await, Decimal::ZERO);
    let a2 = t.insert(&manager, "AssetValueAdjustment", adjust("400")).await;
    let a2 = uid(&t.submit(&manager, "AssetValueAdjustment", uid(&a2)).await);
    // The manual plan shrinks with the value it depreciates: 400 → 200 halves its last row.
    let halve = t
        .insert(
            &manager,
            "AssetValueAdjustment",
            json!({"asset": lathe.to_string(), "adjustment_date": "2024-06-30", "new_asset_value": "200", "difference_account": admin_expenses.to_string()}),
        )
        .await;
    let halve = t.submit(&manager, "AssetValueAdjustment", uid(&halve)).await;
    assert_eq!((dec(&halve["current_asset_value"]), dec(&halve["difference_amount"])), (n("400"), n("-200")));
    let la = t.get("Asset", lathe).await;
    assert_eq!(schedule(&la), [("2022-12-31".into(), n("300"), true), ("2023-12-31".into(), n("300"), true), ("2024-12-31".into(), n("100"), false)]);

    // ───────────── a repair: out of order while pending, capitalized when done ─────────────
    let mine = t
        .insert(
            &bob,
            "AssetRepair",
            json!({"asset": laptop3.to_string(), "failure_date": "2024-01-10", "description": "Screen cracked", "repair_cost": "600",
                                            "capitalize_repair_cost": true, "expense_account": admin_expenses.to_string()}),
        )
        .await;
    assert_eq!(t.get("Asset", laptop3).await["status"], json!("Out of Order"));
    let not_mine = t.k.insert(&bob, "AssetRepair", obj(json!({"asset": laptop1.to_string(), "failure_date": "2024-01-10"}))).await;
    assert!(not_mine.is_err(), "a custodian repairs only their own assets: {not_mine:?}");
    let pending = t.k.submit(&manager, "AssetRepair", uid(&mine), None).await;
    assert!(matches!(pending, Err(ref e) if validation(e, "complete or cancel")), "{pending:?}");
    t.update(&bob, "AssetRepair", uid(&mine), json!({"repair_status": "Completed", "completion_date": "2024-01-15"})).await;
    assert_eq!(t.get("Asset", laptop3).await["status"], json!("Submitted"), "no longer out of order");
    let by_bob = t.k.submit(&bob, "AssetRepair", uid(&mine), None).await;
    assert!(matches!(by_bob, Err(Error::Forbidden(_))), "{by_bob:?}");
    let repair = uid(&t.submit(&manager, "AssetRepair", uid(&mine)).await);
    let l3 = t.get("Asset", laptop3).await;
    let s3 = schedule(&l3);
    assert_eq!(s3.iter().filter(|r| r.2).count(), 12, "2023 booked before the repair");
    let rest: Vec<Decimal> = s3.iter().filter(|r| !r.2).map(|r| r.1).collect();
    // 1200 − 384.91 + 600 = 1415.09 over 24 months and 14/31 of one.
    assert!(rest.len() == 25 && rest[..24].iter().all(|x| *x == n("57.87")) && rest[24] == n("26.21"), "{rest:?}");
    assert_eq!((dec(&l3["additional_asset_cost"]), dec(&l3["total_asset_cost"]), l3["status"].clone()), (n("600"), n("1800"), json!("Partially Depreciated")));
    assert_eq!(t.gl_for("Computers", &[repair]).await, n("600"), "the cost moved from the expense into the asset");

    // ───────────── movements: custody history; a custodian sees what they hold ─────────────
    let same = t
        .k
        .insert(&manager, "AssetMovement", obj(json!({"company": cid.to_string(), "purpose": "Transfer", "transaction_date": "2024-02-01", "assets": [{"asset": laptop3.to_string(), "target_location": floor1.to_string()}]})))
        .await;
    assert!(matches!(same, Err(ref e) if validation(e, "different location")), "{same:?}");
    let transfer = t
        .insert(&manager, "AssetMovement", json!({"company": cid.to_string(), "purpose": "Transfer", "transaction_date": "2024-02-01", "assets": [{"asset": laptop3.to_string(), "target_location": floor2.to_string()}]}))
        .await;
    assert_eq!(rows(&transfer, "assets")[0]["source_location"], json!(floor1.to_string()), "filled in");
    let transfer = uid(&t.submit(&manager, "AssetMovement", uid(&transfer)).await);
    let l3 = t.get("Asset", laptop3).await;
    assert_eq!((l3["location"].clone(), l3["custodian"].clone()), (json!(floor2.to_string()), json!("bob@acme.test")), "a transfer keeps the custodian");
    let issue = t
        .insert(&manager, "AssetMovement", json!({"company": cid.to_string(), "purpose": "Issue", "transaction_date": "2024-03-01", "assets": [{"asset": laptop3.to_string(), "to_custodian": "alice@acme.test"}]}))
        .await;
    let issue = uid(&t.submit(&manager, "AssetMovement", uid(&issue)).await);
    let l3 = t.get("Asset", laptop3).await;
    assert_eq!((l3["location"].clone(), l3["custodian"].clone()), (json!(floor2.to_string()), json!("alice@acme.test")));
    let backdated = t
        .k
        .insert(&manager, "AssetMovement", obj(json!({"company": cid.to_string(), "purpose": "Transfer", "transaction_date": "2024-02-15", "assets": [{"asset": laptop3.to_string(), "target_location": plant.to_string()}]})))
        .await;
    assert!(matches!(backdated, Err(ref e) if validation(e, "moved later")), "{backdated:?}");
    let early_cancel = t.k.cancel(&manager, "AssetMovement", transfer, None).await;
    assert!(matches!(early_cancel, Err(Error::Invalid(ref m)) if m.contains("moved again")), "{early_cancel:?}");
    let seen = |ctx: Ctx| {
        let t = &t;
        async move {
            let mut ids: Vec<Uuid> = t.list(&ctx, "Asset", json!({})).await.iter().map(uid).collect();
            ids.sort();
            ids
        }
    };
    let mut alices = vec![laptop1, laptop3];
    alices.sort();
    assert_eq!(seen(alice.clone()).await, alices, "alice sees the laptops in her custody");
    assert!(seen(bob.clone()).await.is_empty(), "bob handed his laptop over");
    assert!(matches!(t.k.get(&bob, "Asset", laptop3, false).await, Err(Error::NotFound(_))));
    t.cancel(&manager, "AssetMovement", issue).await;
    assert_eq!(t.get("Asset", laptop3).await["custodian"], json!("bob@acme.test"), "cancelling the issue gives it back");
    assert_eq!(seen(bob.clone()).await, vec![laptop3]);
    let issue = t
        .insert(&manager, "AssetMovement", json!({"company": cid.to_string(), "purpose": "Issue", "transaction_date": "2024-03-01", "assets": [{"asset": laptop3.to_string(), "to_custodian": "alice@acme.test"}]}))
        .await;
    t.submit(&manager, "AssetMovement", uid(&issue)).await;
    let mut history =
        t.report(&a, "asset_movement_history", &[("company", cid.to_string()), ("from_date", "2024-01-01".into()), ("to_date", "2024-12-31".into())]).await;
    assert_eq!(history["totals"]["count"], json!(2), "the cancelled issue is not history");
    let first_move = history["rows"][0].take();
    assert_eq!((first_move["from_location"].clone(), first_move["to_location"].clone()), (json!("HQ Floor 1"), json!("HQ Floor 2")));
    assert_eq!(history["rows"][1]["to_custodian"], json!("alice@acme.test"));

    // ───────────── identity facts stay editable after submit ─────────────
    let edit = t.k.update(&manager, "Asset", laptop2, obj(json!({"serial_number": "SN-2"})), None).await;
    assert!(matches!(edit, Err(Error::Invalid(ref m)) if m.contains("cannot be edited")), "a submitted asset is read-only: {edit:?}");
    let details = json!({"serial_number": " SN-2 ", "manufacturer": "Dell", "model_number": "XPS 13", "warranty_expiry_date": "2026-01-14"});
    t.k.run_action(&manager, "Asset", laptop2, "update_details", details).await.expect("edit details");
    let l2 = t.get("Asset", laptop2).await;
    assert_eq!(
        [&l2["serial_number"], &l2["manufacturer"], &l2["model_number"], &l2["warranty_expiry_date"], &l2["end_of_life_date"]],
        [&json!("SN-2"), &json!("Dell"), &json!("XPS 13"), &json!("2026-01-14"), &Value::Null],
        "trimmed, and kept"
    );
    t.k.run_action(&manager, "Asset", laptop2, "update_details", json!({"manufacturer": "", "end_of_life_date": "2027-12-31"})).await.expect("clear");
    let l2 = t.get("Asset", laptop2).await;
    assert_eq!(
        [&l2["manufacturer"], &l2["serial_number"], &l2["end_of_life_date"]],
        [&Value::Null, &json!("SN-2"), &json!("2027-12-31")],
        "empty clears what was there; what is left out stays"
    );
    let bad_date = t.k.run_action(&manager, "Asset", laptop2, "update_details", json!({"warranty_expiry_date": "soon"})).await;
    assert!(matches!(bad_date, Err(ref e) if validation(e, "warranty_expiry_date")), "{bad_date:?}");
    let too_long = t.k.run_action(&manager, "Asset", laptop2, "update_details", json!({"serial_number": "x".repeat(121)})).await;
    assert!(matches!(too_long, Err(ref e) if validation(e, "120")), "{too_long:?}");
    let by_alice = t.k.run_action(&alice, "Asset", laptop3, "update_details", json!({"serial_number": "X"})).await;
    assert!(by_alice.is_err(), "a custodian does not rewrite the register's facts: {by_alice:?}");

    // ───────────── handed out until a date; the date follows the custody ─────────────
    let issue_to = |on: &str, due: Value| {
        json!({"company": cid.to_string(), "purpose": "Issue", "transaction_date": on,
               "assets": [{"asset": laptop2.to_string(), "to_custodian": "bob@acme.test", "expected_return_date": due}]})
    };
    let too_early = t.k.insert(&manager, "AssetMovement", obj(issue_to("2024-03-05", json!("2024-03-01")))).await;
    assert!(matches!(too_early, Err(ref e) if validation(e, "before the day")), "{too_early:?}");
    let on_transfer =
        t.k.insert(
            &manager,
            "AssetMovement",
            obj(json!({"company": cid.to_string(), "purpose": "Transfer", "transaction_date": "2024-03-05",
                       "assets": [{"asset": laptop2.to_string(), "target_location": floor2.to_string(), "expected_return_date": "2024-04-01"}]})),
        )
        .await;
    assert!(matches!(on_transfer, Err(ref e) if validation(e, "only an issue")), "{on_transfer:?}");
    let out = t.insert(&manager, "AssetMovement", issue_to("2024-03-05", json!("2024-04-01"))).await;
    t.submit(&manager, "AssetMovement", uid(&out)).await;
    let l2 = t.get("Asset", laptop2).await;
    assert_eq!((&l2["custodian"], &l2["expected_return_date"]), (&json!("bob@acme.test"), &json!("2024-04-01")));
    let due_back = t.list(&a, "Asset", json!([["expected_return_date", "<", "2024-04-02"]])).await;
    assert_eq!(due_back.iter().map(uid).collect::<Vec<_>>(), vec![laptop2], "the one handed out is the one due back");
    let trail = t
        .report(
            &a,
            "asset_movement_history",
            &[("company", cid.to_string()), ("asset", laptop2.to_string()), ("from_date", "2024-01-01".into()), ("to_date", "2024-12-31".into())],
        )
        .await;
    assert_eq!(
        (&trail["rows"][0]["to_custodian"], &trail["rows"][0]["due_back"]),
        (&json!("bob@acme.test"), &json!("2024-04-01")),
        "the history says when it was due back"
    );
    let receipt = |on: &str| json!({"company": cid.to_string(), "purpose": "Receipt", "transaction_date": on, "assets": [{"asset": laptop2.to_string(), "target_location": floor1.to_string()}]});
    let back = uid(&t.submit(&manager, "AssetMovement", uid(&t.insert(&manager, "AssetMovement", receipt("2024-04-10")).await)).await);
    let l2 = t.get("Asset", laptop2).await;
    assert_eq!((&l2["custodian"], &l2["expected_return_date"]), (&Value::Null, &Value::Null), "handed back: nobody holds it, nothing is due");
    t.cancel(&manager, "AssetMovement", back).await;
    let l2 = t.get("Asset", laptop2).await;
    assert_eq!((&l2["custodian"], &l2["expected_return_date"]), (&json!("bob@acme.test"), &json!("2024-04-01")), "undoing the receipt gives back both");
    let handover = json!({"company": cid.to_string(), "purpose": "Transfer", "transaction_date": "2024-04-15",
                          "assets": [{"asset": laptop2.to_string(), "target_location": floor2.to_string(), "to_custodian": "alice@acme.test"}]});
    let handover = uid(&t.submit(&manager, "AssetMovement", uid(&t.insert(&manager, "AssetMovement", handover).await)).await);
    let l2 = t.get("Asset", laptop2).await;
    assert_eq!((&l2["custodian"], &l2["expected_return_date"]), (&json!("alice@acme.test"), &Value::Null), "a new custodian inherits no date");
    t.cancel(&manager, "AssetMovement", handover).await;
    let l2 = t.get("Asset", laptop2).await;
    assert_eq!((&l2["custodian"], &l2["expected_return_date"]), (&json!("bob@acme.test"), &json!("2024-04-01")));
    t.submit(&manager, "AssetMovement", uid(&t.insert(&manager, "AssetMovement", receipt("2024-05-01")).await)).await;
    let l2 = t.get("Asset", laptop2).await;
    assert_eq!((&l2["custodian"], &l2["expected_return_date"]), (&Value::Null, &Value::Null), "and it stays in the register for what follows");

    // ───────────── seen in person ─────────────
    t.update(&a, "AssetCategory", laptops, json!({"verification_interval_months": 6})).await;
    let check = |asset: Uuid, on: &str, more: Value| {
        let mut v = json!({"asset": asset.to_string(), "verified_on": on});
        for (k, x) in more.as_object().unwrap() {
            v[k] = x.clone();
        }
        v
    };
    for (what, input, needle) in [
        ("tomorrow", check(laptop1, "2999-01-01", json!({})), "future"),
        ("before it was bought", check(laptop1, "2022-12-31", json!({})), "bought"),
        ("a next check before the check", check(laptop1, "2024-05-01", json!({"next_verification_date": "2024-05-01"})), "after the day"),
        ("seen and not found", check(laptop1, "2024-05-01", json!({"result": "Not Found", "seen_at": floor1.to_string()})), "seen nowhere"),
    ] {
        let refused = t.k.insert(&manager, "AssetVerification", obj(input)).await;
        assert!(matches!(refused, Err(ref e) if validation(e, needle)), "{what}: {refused:?}");
    }
    let by_alice = t.k.insert(&alice, "AssetVerification", obj(check(laptop1, "2024-05-01", json!({})))).await;
    assert!(by_alice.is_err(), "checking is the asset manager's job: {by_alice:?}");
    let v1 = t.insert(&manager, "AssetVerification", check(laptop1, "2024-05-01", json!({"seen_at": floor1.to_string(), "note": "On the desk"}))).await;
    assert_eq!(
        (&v1["company"], &v1["verified_by"], &v1["next_verification_date"]),
        (&json!(cid.to_string()), &json!("manager@acme.test"), &json!("2024-11-01")),
        "the company from the asset, the person signed in, the next check from the category (6 months)"
    );
    let l1 = t.get("Asset", laptop1).await;
    assert!(l1["last_verified_date"].is_null(), "a draft check has not happened yet");
    t.submit(&manager, "AssetVerification", uid(&v1)).await;
    let verified = |l: &Doc| (l["last_verified_date"].clone(), l["next_verification_date"].clone(), l["verification_result"].clone());
    assert_eq!(verified(&t.get("Asset", laptop1).await), (json!("2024-05-01"), json!("2024-11-01"), json!("Found")));
    let lost = t.insert(&manager, "AssetVerification", check(laptop1, "2024-06-01", json!({"result": "Not Found"}))).await;
    assert!(lost["next_verification_date"].is_null(), "an asset that cannot be found is not scheduled for a next check");
    t.submit(&manager, "AssetVerification", uid(&lost)).await;
    assert_eq!(verified(&t.get("Asset", laptop1).await), (json!("2024-06-01"), Value::Null, json!("Not Found")), "the latest check says so");
    t.cancel(&manager, "AssetVerification", uid(&lost)).await;
    assert_eq!(
        verified(&t.get("Asset", laptop1).await),
        (json!("2024-05-01"), json!("2024-11-01"), json!("Found")),
        "taking a check back restores the one before"
    );
    let given = t.insert(&manager, "AssetVerification", check(laptop1, "2024-07-01", json!({"next_verification_date": "2025-01-15"}))).await;
    t.submit(&manager, "AssetVerification", uid(&given)).await;
    assert_eq!(verified(&t.get("Asset", laptop1).await), (json!("2024-07-01"), json!("2025-01-15"), json!("Found")), "a date given beats the category's");
    let van_check = t.insert(&manager, "AssetVerification", check(van, "2024-05-02", json!({}))).await;
    assert!(van_check["next_verification_date"].is_null(), "a category with no interval has no routine check");
    t.submit(&manager, "AssetVerification", uid(&van_check)).await;
    assert_eq!(verified(&t.get("Asset", van).await), (json!("2024-05-02"), Value::Null, json!("Found")));
    let due = t.list(&a, "Asset", json!([["next_verification_date", "<=", "2025-02-01"]])).await;
    assert_eq!(due.iter().map(uid).collect::<Vec<_>>(), vec![laptop1], "the due list holds what was scheduled and is due");

    // ───────────── scrap: pending depreciation first, then the write-off; undone by cancelling ─────────────
    let future =
        t.k.insert(&manager, "AssetDisposal", obj(json!({"asset": boiler.to_string(), "disposal_type": "Scrap", "disposal_date": "2999-01-01"}))).await;
    assert!(matches!(future, Err(ref e) if validation(e, "future")), "{future:?}");
    let handed = json!({"company": cid.to_string(), "purpose": "Issue", "transaction_date": "2024-07-01",
                        "assets": [{"asset": boiler.to_string(), "to_custodian": "bob@acme.test", "expected_return_date": "2024-09-01"}]});
    t.submit(&manager, "AssetMovement", uid(&t.insert(&manager, "AssetMovement", handed).await)).await;
    let watched = t
        .insert(&manager, "AssetVerification", json!({"asset": boiler.to_string(), "verified_on": "2024-07-02", "next_verification_date": "2025-07-02"}))
        .await;
    t.submit(&manager, "AssetVerification", uid(&watched)).await;
    let dates =
        |l: &Doc| [l["expected_return_date"].clone(), l["last_verified_date"].clone(), l["verification_result"].clone(), l["next_verification_date"].clone()];
    assert_eq!(dates(&t.get("Asset", boiler).await), [json!("2024-09-01"), json!("2024-07-02"), json!("Found"), json!("2025-07-02")]);
    let scrap = t.insert(&manager, "AssetDisposal", json!({"asset": boiler.to_string(), "disposal_type": "Scrap", "disposal_date": "2024-07-15"})).await;
    let scrap = t.submit(&manager, "AssetDisposal", uid(&scrap)).await;
    assert_eq!(
        dates(&t.get("Asset", boiler).await),
        [Value::Null, json!("2024-07-02"), Value::Null, Value::Null],
        "a scrapped asset is due nothing and found nothing, but the day it was last verified stays"
    );
    let after_scrap = t.k.insert(&manager, "AssetVerification", obj(json!({"asset": boiler.to_string(), "verified_on": "2024-07-20"}))).await;
    assert!(matches!(after_scrap, Err(ref e) if validation(e, "scrapped")), "{after_scrap:?}");
    // 2023: 50000 × 351/365 / 10 = 4808.22; 2024 to Jul 15: 5000 × 197/366 = 2691.26.
    assert_eq!(
        (dec(&scrap["accumulated_depreciation"]), dec(&scrap["net_book_value"]), dec(&scrap["gain_or_loss"])),
        (n("7499.48"), n("42500.52"), n("-42500.52"))
    );
    let bo = t.get("Asset", boiler).await;
    assert_eq!((bo["status"].clone(), bo["disposal_date"].clone()), (json!("Scrapped"), json!("2024-07-15")));
    assert_eq!(schedule(&bo), [("2023-12-31".into(), n("4808.22"), true), ("2024-07-15".into(), n("2691.26"), true)], "the plan after the scrap is gone");
    assert_eq!(t.gl_for("Plant and Machinery", &[uid(&scrap)]).await, n("-50000"));
    assert_eq!(t.gl_for("Gain/Loss on Asset Disposal", &[uid(&scrap)]).await, n("42500.52"));
    let moved =
        t.k.insert(
            &manager,
            "AssetMovement",
            obj(json!({"company": cid.to_string(), "purpose": "Transfer", "assets": [{"asset": boiler.to_string(), "target_location": floor1.to_string()}]})),
        )
        .await;
    assert!(matches!(moved, Err(ref e) if validation(e, "scrapped")), "a scrapped asset stays put: {moved:?}");
    let plant_before = t.gl("Plant and Machinery").await;
    t.cancel(&manager, "AssetDisposal", uid(&scrap)).await;
    let bo = t.get("Asset", boiler).await;
    assert_eq!(bo["status"], json!("Partially Depreciated"));
    assert_eq!(dates(&bo), [json!("2024-09-01"), json!("2024-07-02"), json!("Found"), json!("2025-07-02")], "undoing the scrap brings its dates back");
    let take_back = json!({"company": cid.to_string(), "purpose": "Receipt", "transaction_date": "2024-08-02", "assets": [{"asset": boiler.to_string(), "target_location": plant.to_string()}]});
    t.submit(&manager, "AssetMovement", uid(&t.insert(&manager, "AssetMovement", take_back).await)).await;
    assert!(t.get("Asset", boiler).await["expected_return_date"].is_null(), "handed back");
    let sb = schedule(&bo);
    assert_eq!(sb.len(), 11, "the plan is back");
    assert!(sb[0].2 && sb[1..].iter().all(|r| !r.2) && sb[1].1 == n("5000") && sb[10].1 == n("191.78"));
    assert_eq!(t.gl("Plant and Machinery").await - plant_before, n("50000"));

    // ───────────── sale at a loss through a sales invoice; undone; redone at a gain ─────────────
    let sale = t
        .insert(&manager, "AssetDisposal", json!({"asset": chairs.to_string(), "disposal_type": "Sell", "disposal_date": "2025-03-31", "customer": customer.to_string(), "sale_amount": "450"}))
        .await;
    let direct = t.k.submit(&manager, "AssetDisposal", uid(&sale), None).await;
    assert!(matches!(direct, Err(Error::Invalid(ref m)) if m.contains("sales invoice")), "{direct:?}");
    let made = t.k.run_action(&manager, "AssetDisposal", uid(&sale), "make_sales_invoice", json!({})).await.expect("make invoice");
    let si1: Uuid = made["id"].as_str().unwrap().parse().unwrap();
    let again = t.k.run_action(&manager, "AssetDisposal", uid(&sale), "make_sales_invoice", json!({})).await.unwrap();
    assert_eq!(again["id"], made["id"], "the same draft invoice");
    t.update(&manager, "AssetDisposal", uid(&sale), json!({"sale_amount": "500"})).await;
    let inv = t.get("SalesInvoice", si1).await;
    let line = &rows(&inv, "items")[0];
    assert_eq!((dec(&line["price_list_rate"]), line["income_account"].clone()), (n("500"), json!(furniture.to_string())), "the invoice follows the disposal");
    assert_eq!((dec(&inv["grand_total"]), inv["posting_date"].clone()), (n("500"), json!("2025-03-31")));
    t.submit(&manager, "SalesInvoice", si1).await;
    let sale = t.get("AssetDisposal", uid(&sale)).await;
    assert_eq!(sale["docstatus"], json!(1), "the invoice submitted the disposal");
    // 288.49 (2023) + 300 (2024) + 300 × 90/365 (2025 to Mar 31) = 662.46.
    assert_eq!(
        (dec(&sale["accumulated_depreciation"]), dec(&sale["net_book_value"]), dec(&sale["proceeds"]), dec(&sale["gain_or_loss"])),
        (n("662.46"), n("837.54"), n("500"), n("-337.54"))
    );
    assert_eq!(t.get("Asset", chairs).await["status"], json!("Sold"));
    assert_eq!(t.gl_for("Furniture and Fixtures", &[si1, uid(&sale)]).await, n("-1500"), "the chairs are out of the books");
    assert_eq!(t.gl_for("Gain/Loss on Asset Disposal", &[uid(&sale)]).await, n("337.54"), "a loss");
    let undo = t.k.cancel(&manager, "AssetDisposal", uid(&sale), None).await;
    assert!(matches!(undo, Err(Error::Invalid(ref m)) if m.contains("cancel sales invoice")), "{undo:?}");
    let credit = t.k.run_action(&a, "SalesInvoice", si1, "make_credit_note", json!({})).await;
    assert!(matches!(credit, Err(ref e) if validation(e, "cancelling its invoice")), "{credit:?}");
    t.cancel(&a, "SalesInvoice", si1).await;
    assert_eq!(t.get("AssetDisposal", uid(&sale)).await["docstatus"], json!(2), "cancelling the invoice cancelled the disposal");
    let ch = t.get("Asset", chairs).await;
    assert_eq!(ch["status"], json!("Partially Depreciated"));
    assert_eq!(schedule(&ch).iter().map(|r| r.2).collect::<Vec<_>>(), [true, true, false, false, false, false], "two years booked, the rest planned again");
    assert_eq!(t.gl("Furniture and Fixtures").await, n("1500"));
    let resale = t
        .insert(&manager, "AssetDisposal", json!({"asset": chairs.to_string(), "disposal_type": "Sell", "disposal_date": "2025-06-30", "customer": customer.to_string(), "sale_amount": "1100"}))
        .await;
    let made = t.k.run_action(&manager, "AssetDisposal", uid(&resale), "make_sales_invoice", json!({})).await.unwrap();
    let draft: Uuid = made["id"].as_str().unwrap().parse().unwrap();
    let blocked = t.k.delete(&manager, "AssetDisposal", uid(&resale), None).await;
    assert!(matches!(blocked, Err(Error::Invalid(ref m)) if m.contains("delete draft sales invoice")), "{blocked:?}");
    t.k.delete(&a, "SalesInvoice", draft, None).await.expect("drop the draft invoice");
    assert!(t.get("AssetDisposal", uid(&resale)).await["sales_invoice"].is_null(), "deleting the invoice frees the disposal");
    let made = t.k.run_action(&manager, "AssetDisposal", uid(&resale), "make_sales_invoice", json!({})).await.unwrap();
    let si2: Uuid = made["id"].as_str().unwrap().parse().unwrap();
    assert_ne!(si2, draft);
    t.submit(&manager, "SalesInvoice", si2).await;
    let resale = t.get("AssetDisposal", uid(&resale)).await;
    // 588.49 + 300 × 181/365 = 737.26 accumulated; 1100 − 762.74 = 337.26 gain.
    assert_eq!((dec(&resale["accumulated_depreciation"]), dec(&resale["gain_or_loss"])), (n("737.26"), n("337.26")));
    assert_eq!(t.gl_for("Gain/Loss on Asset Disposal", &[uid(&resale)]).await, n("-337.26"), "a gain");
    assert_eq!(t.gl("Furniture and Fixtures").await, Decimal::ZERO);

    // ───────────── a closed period stops the upkeep at that row; reopening resumes it ─────────────
    let fy =
        t.insert(&a, "FiscalYear", json!({"year_name": "2024-25", "company": cid.to_string(), "start_date": "2024-04-01", "end_date": "2025-03-31"})).await;
    t.k.run_action(&a, "FiscalYear", uid(&fy), "close_period", json!({"period_start": "2024-04-01"})).await.expect("close April 2024");
    let before = t.depreciation_entries().await;
    t.upkeep().await;
    let l3 = t.get("Asset", laptop3).await;
    let stuck = rows(&l3, "schedule").into_iter().find(|r| r["booked"] != json!(true)).unwrap();
    assert_eq!(stuck["schedule_date"], json!("2024-04-30"), "booked up to March");
    assert!(stuck["posting_error"].as_str().unwrap().contains("closed"), "{}", stuck["posting_error"]);
    let notified = |user: Uuid| t.count("select count(*) from notification where tenant_id = $1 and user_id = $2 and title like 'Depreciation%'", Some(user));
    assert_eq!(notified(manager.user).await, 1, "the asset manager hears about it");
    t.upkeep().await;
    assert_eq!(notified(manager.user).await, 1, "once");
    assert!(t.depreciation_entries().await > before, "other assets went on");
    t.k.run_action(&a, "FiscalYear", uid(&fy), "reopen_period", json!({"period_start": "2024-04-01"})).await.unwrap();
    t.upkeep().await;
    let after = t.depreciation_entries().await;
    t.upkeep().await;
    assert_eq!(t.depreciation_entries().await, after, "idempotent");
    let l3 = t.get("Asset", laptop3).await;
    assert!(rows(&l3, "schedule").iter().all(|r| r["booked"] == json!(true) && r["posting_error"].is_null()));
    assert_eq!((dec(&l3["accumulated_depreciation"]), l3["status"].clone()), (n("1800"), json!("Fully Depreciated")), "the repair depreciated too");
    let l2 = t.get("Asset", laptop2).await;
    assert_eq!((dec(&l2["accumulated_depreciation"]), dec(&l2["value_after_depreciation"])), (n("984.89"), n("0")));
    let late = t.k.cancel(&manager, "AssetValueAdjustment", a2, None).await;
    assert!(matches!(late, Err(Error::Invalid(ref m)) if m.contains("no depreciation periods left")), "{late:?}");

    // ───────────── a fully depreciated laptop sold at a gain ─────────────
    let sale = t
        .insert(&manager, "AssetDisposal", json!({"asset": laptop2.to_string(), "disposal_type": "Sell", "disposal_date": "2026-03-15", "customer": customer.to_string(), "sale_amount": "300"}))
        .await;
    let made = t.k.run_action(&manager, "AssetDisposal", uid(&sale), "make_sales_invoice", json!({})).await.unwrap();
    let si3: Uuid = made["id"].as_str().unwrap().parse().unwrap();
    t.submit(&manager, "SalesInvoice", si3).await;
    let sale = t.get("AssetDisposal", uid(&sale)).await;
    assert_eq!((dec(&sale["net_book_value"]), dec(&sale["gain_or_loss"])), (n("0"), n("300")));
    assert_eq!(t.gl_for("Computers", &[pi_id]).await - n("2400"), n("1200"), "laptop 2's cost on the invoice");
    assert_eq!(t.gl_for("Computers", &[uid(&a1), a2, si3, uid(&sale)]).await, n("-1200"), "impairment, sale and disposal take laptop 2 out");

    // ───────────── maintenance: due tasks open a log and notify; done tasks move on ─────────────
    let plan = t
        .insert(&manager, "AssetMaintenance", json!({"asset": laptop3.to_string(), "maintenance_manager": "manager@acme.test", "tasks": [
            {"task_name": "Battery check", "periodicity": "Monthly", "start_date": "2026-01-01", "assign_to": "bob@acme.test"},
            {"task_name": "Recalibrate", "maintenance_type": "Calibration", "periodicity": "Yearly", "start_date": "2026-01-01", "end_date": "2026-06-30", "assign_to": "bob@acme.test"}]}))
        .await;
    let tasks = rows(&plan, "tasks");
    assert_eq!(tasks[0]["next_due_date"], json!("2026-02-01"), "one period after the start");
    assert!(tasks[1]["next_due_date"].is_null(), "the yearly check falls after the end date");
    let plan = uid(&plan);
    t.upkeep().await;
    let logs = t.list(&a, "AssetMaintenanceLog", json!({"asset_maintenance": plan.to_string()})).await;
    assert_eq!(logs.len(), 1);
    assert_eq!(
        (logs[0]["due_date"].clone(), logs[0]["maintenance_status"].clone(), logs[0]["assigned_to"].clone()),
        (json!("2026-02-01"), json!("Planned"), json!("bob@acme.test"))
    );
    let bob_told = t.count("select count(*) from notification where tenant_id = $1 and user_id = $2 and title like 'Maintenance due%'", Some(bob.user)).await;
    assert_eq!(bob_told, 1);
    t.upkeep().await;
    assert_eq!(t.list(&a, "AssetMaintenanceLog", json!({"asset_maintenance": plan.to_string()})).await.len(), 1, "once per due date");
    let log = uid(&logs[0]);
    t.update(&bob, "AssetMaintenanceLog", log, json!({"maintenance_status": "In Progress"})).await;
    assert_eq!(t.get("Asset", laptop3).await["status"], json!("In Maintenance"));
    let busy = t.k.insert(&manager, "AssetDisposal", obj(json!({"asset": laptop3.to_string(), "disposal_type": "Scrap", "disposal_date": "2026-02-02"}))).await;
    assert!(matches!(busy, Err(ref e) if validation(e, "in maintenance")), "{busy:?}");
    t.update(&bob, "AssetMaintenanceLog", log, json!({"maintenance_status": "Completed", "completion_date": "2026-02-03", "actions_performed": "Replaced"}))
        .await;
    t.submit(&bob, "AssetMaintenanceLog", log).await;
    assert_eq!(t.get("Asset", laptop3).await["status"], json!("Fully Depreciated"));
    let tasks = rows(&t.get("AssetMaintenance", plan).await, "tasks");
    assert_eq!((tasks[0]["last_completion_date"].clone(), tasks[0]["next_due_date"].clone()), (json!("2026-02-03"), json!("2026-03-03")));
    let due = t.report(&a, "asset_maintenance_due", &[("company", cid.to_string()), ("as_of", "2026-03-10".into()), ("days", "0".into())]).await;
    assert_eq!(due["rows"].as_array().unwrap().len(), 1);
    assert_eq!((due["rows"][0]["status"].clone(), due["rows"][0]["days_overdue"].clone()), (json!("Overdue"), json!(7)));
    assert_eq!(due["rows"][0]["asset_id"], json!(laptop3.to_string()), "the row says which asset it is for");
    let bobs = t.report(&bob, "asset_maintenance_due", &[("company", cid.to_string()), ("as_of", "2026-03-10".into())]).await;
    assert_eq!(bobs["rows"].as_array().unwrap().len(), 1, "the assignee sees their task");
    let alices = t.report(&alice, "asset_maintenance_due", &[("company", cid.to_string()), ("as_of", "2026-03-10".into())]).await;
    assert!(alices["rows"].as_array().unwrap().is_empty(), "not alice's task");

    // ───────────── cancelling an asset reverses its depreciation ─────────────
    let spare = t
        .insert(
            &a,
            "Asset",
            json!({"item": laptop_item.to_string(), "company": cid.to_string(), "location": floor1.to_string(), "is_existing_asset": true,
                                    "purchase_date": "2025-01-01", "gross_purchase_amount": "360"}),
        )
        .await;
    assert!(amounts(&spare).iter().all(|x| *x == n("10")) && amounts(&spare).len() == 36, "a full first month: no pro-rata");
    let spare = uid(&t.submit(&a, "Asset", uid(&spare)).await);
    t.upkeep().await;
    let booked = schedule(&t.get("Asset", spare).await).iter().filter(|r| r.2).count();
    assert!(booked >= 21, "January 2025 to at least September 2026");
    let dep_before = t.gl("Depreciation").await;
    let spare_check =
        t.insert(&manager, "AssetVerification", json!({"asset": spare.to_string(), "verified_on": "2025-02-01", "next_verification_date": "2026-02-01"})).await;
    t.submit(&manager, "AssetVerification", uid(&spare_check)).await;
    assert_eq!(t.get("Asset", spare).await["next_verification_date"], json!("2026-02-01"));
    t.cancel(&a, "Asset", spare).await;
    let sp = t.get("Asset", spare).await;
    assert!(sp["next_verification_date"].is_null(), "a cancelled asset is due no check");
    assert_eq!((sp["status"].clone(), dec(&sp["accumulated_depreciation"])), (json!("Cancelled"), Decimal::ZERO));
    assert_eq!(dep_before - t.gl("Depreciation").await, n("10") * Decimal::from(booked as i64), "its depreciation is reversed");
    let cancelled_entries =
        t.count("select count(*) from e_journal_entry where tenant_id = $1 and docstatus = 2 and (ext->>'fa_asset')::uuid = $2", Some(spare)).await;
    assert_eq!(cancelled_entries, booked as i64);

    // ───────────── reports ─────────────
    let p = |extra: &[(&'static str, &str)]| -> Vec<(&'static str, String)> {
        let mut v = vec![("company", cid.to_string())];
        v.extend(extra.iter().map(|(k, x)| (*k, x.to_string())));
        v
    };
    let reg = t.report(&accountant, "fixed_asset_register", &p(&[("as_of", "2023-12-31"), ("asset_category", &laptops.to_string())])).await;
    assert_eq!(reg["rows"].as_array().unwrap().len(), 3);
    // Each laptop: 1200 cost, 2023 depreciation 384.91 (the repair and impairment came in 2024).
    assert_eq!(
        (dec(&reg["totals"]["cost"]), dec(&reg["totals"]["accumulated_depreciation"]), dec(&reg["totals"]["net_book_value"])),
        (n("3600"), n("1154.73"), n("2445.27"))
    );
    let floors: Vec<Value> = reg["rows"].as_array().unwrap().iter().map(|r| r["location"].clone()).collect();
    assert!(floors.iter().all(|f| *f == json!("HQ Floor 1")), "laptop 3 was still on floor 1 then: {floors:?}");
    let by_category = t.report(&accountant, "fixed_asset_register", &p(&[("as_of", "2024-06-30"), ("group_by", "Asset Category")])).await;
    let plant_row = by_category["rows"].as_array().unwrap().iter().find(|r| r["asset_category"] == json!("Plant")).unwrap().clone();
    assert_eq!((plant_row["assets"].clone(), dec(&plant_row["cost"])), (json!(1), n("50000")), "the boiler's scrap was undone");
    let today = chrono::Utc::now().date_naive().to_string();
    let now = t.report(&a, "fixed_asset_register", &p(&[("as_of", &today)])).await;
    let held = ["Computers", "Plant and Machinery", "Furniture and Fixtures", "Vehicles"];
    let mut ledger_cost = Decimal::ZERO;
    for acc in held {
        ledger_cost += t.gl(acc).await;
    }
    assert_eq!(dec(&now["totals"]["cost"]), ledger_cost, "the register ties to the fixed asset accounts");
    assert_eq!(dec(&now["totals"]["accumulated_depreciation"]), -t.gl("Accumulated Depreciation").await, "and to accumulated depreciation");
    assert_eq!(t.gl("Capital Work in Progress").await, Decimal::ZERO);
    let mine = t.report(&alice, "fixed_asset_register", &p(&[("as_of", &today)])).await;
    let names: Vec<Value> = mine["rows"].as_array().unwrap().iter().map(|r| r["asset_id"].clone()).collect();
    assert_eq!(names.len(), 2, "alice's register lists her laptops only");
    let sched =
        t.report(&a, "asset_depreciation_schedule", &p(&[("from_date", "2023-01-01"), ("to_date", "2023-12-31"), ("asset", &laptop1.to_string())])).await;
    assert_eq!(sched["rows"].as_array().unwrap().len(), 12);
    assert_eq!((dec(&sched["totals"]["planned_amount"]), dec(&sched["totals"]["booked_amount"])), (n("384.91"), n("384.91")));
    let boiler_plan =
        t.report(&a, "asset_depreciation_schedule", &p(&[("from_date", "2024-01-01"), ("to_date", "2099-12-31"), ("asset", &boiler.to_string())])).await;
    assert!(boiler_plan["rows"].as_array().unwrap().iter().any(|r| r["status"] == json!("Planned")), "planned against booked");
    let gains = t.report(&accountant, "asset_disposal_gain_loss", &p(&[("from_date", "2023-01-01"), ("to_date", &today)])).await;
    assert_eq!(gains["rows"].as_array().unwrap().len(), 2, "the sold chairs and laptop; the undone scrap and sale are gone");
    assert_eq!((dec(&gains["totals"]["gains"]), dec(&gains["totals"]["losses"])), (n("637.26"), Decimal::ZERO));
    assert_eq!(dec(&gains["totals"]["gain_or_loss"]), -t.gl("Gain/Loss on Asset Disposal").await, "the report ties to the gain/loss account");
    let denied = ferrum_domain::reports::run(&t.k, &bob, "asset_disposal_gain_loss", &HashMap::new()).await;
    assert!(matches!(denied, Err(Error::Forbidden(_))), "{denied:?}");

    // ───────────── the books ─────────────
    let tb = t.report(&a, "trial_balance", &[("company", cid.to_string()), ("from_date", "2019-01-01".into()), ("to_date", today.clone())]).await;
    assert_eq!(tb["meta"]["balanced"], json!(true), "{}", tb["totals"]);
    let unbalanced = t
        .count("select count(*) from (select entry_id from journal_line where tenant_id = $1 and $2::uuid is null group by entry_id having sum(debit) <> sum(credit)) x", None)
        .await;
    assert_eq!(unbalanced, 0);
    let refused = t.k.cancel(&a, "Asset", laptop2, None).await;
    assert!(matches!(refused, Err(Error::Invalid(ref m)) if m.contains("disposal")), "a sold asset is not cancelled: {refused:?}");
    let _ = (alice, press, van);
}
