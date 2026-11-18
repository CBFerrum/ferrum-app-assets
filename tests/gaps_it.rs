//! The gap-closing slices' scenario against a real PostgreSQL (`FERRUM_ASSETS_GAPS_TEST_DATABASE_URL`,
//! default `postgres://ferrum:ferrum@localhost:5432/ferrum_assets_gaps_test`). The database is reset
//! first. One company, then one section per gap, in order; each section's figures are to the cent.

use ferrum_kernel::capability::Invocation;
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

/// `(date, amount, booked)` of an asset's own schedule (`None`) or a finance book's.
fn plan(doc: &Doc, book: Option<Uuid>) -> Vec<(String, Decimal, bool)> {
    let (table, book) = match book {
        None => ("schedule", None),
        Some(b) => ("book_schedule", Some(json!(b.to_string()))),
    };
    rows(doc, table)
        .iter()
        .filter(|r| book.as_ref().is_none_or(|b| r["finance_book"] == *b))
        .map(|r| (r["schedule_date"].as_str().unwrap().to_string(), dec(&r["depreciation_amount"]), r["booked"] == json!(true)))
        .collect()
}

fn unbooked(doc: &Doc, book: Option<Uuid>) -> Decimal {
    plan(doc, book).iter().filter(|r| !r.2).map(|r| r.1).sum()
}

fn booked(doc: &Doc, book: Option<Uuid>) -> Vec<Decimal> {
    plan(doc, book).iter().filter(|r| r.2).map(|r| r.1).collect()
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
    async fn insert(&self, entity: &str, v: Value) -> Doc {
        self.k.insert(&self.admin, entity, obj(v)).await.unwrap_or_else(|e| panic!("insert {entity}: {e:?}"))
    }
    async fn submit(&self, entity: &str, id: Uuid) -> Doc {
        self.k.submit(&self.admin, entity, id, None).await.unwrap_or_else(|e| panic!("submit {entity}: {e:?}"))
    }
    async fn cancel(&self, entity: &str, id: Uuid) -> Doc {
        self.k.cancel(&self.admin, entity, id, None).await.unwrap_or_else(|e| panic!("cancel {entity}: {e:?}"))
    }
    async fn get(&self, entity: &str, id: Uuid) -> Doc {
        self.k.get(&self.admin, entity, id, false).await.unwrap_or_else(|e| panic!("get {entity}: {e:?}"))
    }
    async fn list(&self, entity: &str, filters: &Value) -> Vec<Doc> {
        let q = ListQuery { filters: ferrum_kernel::query::parse_filters(filters).unwrap(), limit: Some(500), ..Default::default() };
        self.k.list(&self.admin, entity, &q).await.unwrap_or_else(|e| panic!("list {entity}: {e:?}")).rows
    }
    async fn find_one(&self, entity: &str, filters: Value) -> Doc {
        let found = self.list(entity, &filters).await;
        assert_eq!(found.len(), 1, "{entity} {filters}");
        found[0].clone()
    }
    /// Submitted depreciation entries of one asset.
    async fn entries(&self, asset: Uuid) -> i64 {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        sqlx::query_scalar(
            "select count(*) from e_journal_entry where tenant_id = $1 and entry_type = 'Depreciation' and docstatus = 1 and ext->>'fa_asset' = $2::text",
        )
        .bind(self.tenant)
        .bind(asset)
        .fetch_one(&mut *tx)
        .await
        .unwrap()
    }
    /// Each booked row of the asset's own schedule posts one entry and takes it out of every finance
    /// book that keeps its own (one entry each); each booked row of a book posts one.
    async fn posted_once(&self, asset: Uuid) {
        let doc = self.get("Asset", asset).await;
        let books = rows(&doc, "finance_books").len();
        let own = plan(&doc, None).iter().filter(|r| r.2).count();
        let in_books = rows(&doc, "book_schedule").iter().filter(|r| r["booked"] == json!(true)).count();
        assert_eq!(self.entries(asset).await, (own * (1 + books) + in_books) as i64, "one entry per booked row, never two");
    }
    async fn user(&self, email: &str, role: &str) -> Ctx {
        let id = self.k.create_user(&self.admin, email, email, Some("Correct-Horse-9"), &[role.into()], false).await.expect("user");
        self.k.load_ctx(self.tenant, id, None, "it".into()).await.expect("ctx")
    }
    async fn usage(&self, ctx: &Ctx, asset: Uuid, period_end: &str, units: &str) -> Result<Value, Error> {
        self.k.run_action(ctx, "Asset", asset, "record_usage", json!({"period_end": period_end, "units": units})).await
    }
    async fn upkeep(&self) {
        ferrum_app_assets::maintenance(&self.k, self.tenant).await.expect("asset upkeep");
    }
}

struct Setup {
    company: Uuid,
    category: Uuid,
    item: Uuid,
    location: Uuid,
    tax: Uuid,
}

#[test]
fn gaps_end_to_end() {
    // Same runtime shape as the server: unoptimized async hook chains need more than 2 MiB.
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url =
        std::env::var("FERRUM_ASSETS_GAPS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_assets_gaps_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 10, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_ops::packages().expect("ops"));
    packages.extend(ferrum_app_erp::packages().expect("ERP's packages"));
    packages.extend(ferrum_app_assets::packages().expect("app packages"));
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    hooks.app("erp", &[], ferrum_app_erp::register);
    for source in ferrum_app_erp::reports() {
        ferrum_domain::reports::add_source(source);
    }
    ferrum_app_assets::register(&mut hooks);
    for source in ferrum_app_assets::reports() {
        ferrum_domain::reports::add_source(source);
    }
    let k = Kernel::boot(db, catalog, hooks, KernelSettings::default(), None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "acme".into(),
            name: "Acme Industries".into(),
            admin_email: "admin@acme.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["assets".into(), "assets_us".into()],
            settings: Some(json!({"fiscal_year_start_month": 4})),
        })
        .await
        .expect("tenant");
    let session = k.login("acme", "admin@acme.test", "Correct-Horse-9").await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let t = T { k: k.clone(), tenant, admin };

    // A company (its asset accounts come with it), its years from 2019, a category, an item, a place.
    let inr = t.find_one("Currency", json!({"code": "INR"})).await;
    let company = uid(&t.insert("Company", json!({"company_name": "Acme Industries", "abbr": "AI", "default_currency": inr["id"]})).await);
    for y in 2019..=2025 {
        if t.list("FiscalYear", &json!({"company": company.to_string(), "start_date": format!("{y}-04-01")})).await.is_empty() {
            t.insert("FiscalYear", json!({"year_name": format!("{y}-{:02}", (y + 1) % 100), "company": company.to_string(), "start_date": format!("{y}-04-01"), "end_date": format!("{}-03-31", y + 1)}))
                .await;
        }
    }
    let furniture = uid(&t.find_one("Account", json!({"account_name": "Furniture and Fixtures", "company": company.to_string()})).await);
    let category = uid(&t
        .insert(
            "AssetCategory",
            json!({"category_name": "Furniture", "total_number_of_depreciations": 6, "accounts": [{"company": company.to_string(), "fixed_asset_account": furniture.to_string()}]}),
        )
        .await);
    let products = t.find_one("ItemGroup", json!({"group_name": "Products"})).await;
    let nos = t.find_one("Uom", json!({"uom_name": "Nos"})).await;
    let item = uid(&t
        .insert(
            "Item",
            json!({"item_code": "DESK", "item_name": "desk", "item_group": products["id"], "stock_uom": nos["id"], "is_stock_item": false,
                   "fa_is_fixed_asset": true, "fa_asset_category": category.to_string()}),
        )
        .await);
    let location = uid(&t.insert("Location", json!({"location_name": "Office"})).await);
    let tax = uid(&t.insert("FinanceBook", json!({"finance_book_name": "Tax"})).await);
    let s = Setup { company, category, item, location, tax };

    a_split_leaves_each_plan_exact(&t, &s).await;
    let mill = a_half_year_book_beside_a_common_one(&t, &s).await;
    a_convention_must_fit_its_periods(&t, &s).await;
    leaving_under_half_year_takes_half_the_year(&t, &s, mill).await;
    sum_of_years_on_an_asset_and_in_a_book(&t, &s).await;
    usage_becomes_depreciation(&t, &s).await;
    let forklift = usage_through_the_gate_repeats_safely(&t, &s).await;
    who_may_record_usage(&t, forklift, mill).await;
    a_revaluation_between_usages_is_carried(&t, &s, forklift).await;
    the_seeded_tables_total_100(&t).await;
    a_tax_book_depreciates_by_a_table(&t, &s).await;
    a_table_changed_later_leaves_a_submitted_asset_alone(&t, &s).await;
    a_table_must_total_100_and_run_without_gaps(&t).await;
    an_asset_user_cannot_make_a_table(&t).await;
    a_table_needs_yearly_periods(&t, &s).await;
}

/// An asset already in the books, from `json` over the setup's defaults.
fn asset(s: &Setup, extra: Value) -> Value {
    let mut v = json!({"item": s.item.to_string(), "asset_category": s.category.to_string(), "company": s.company.to_string(), "location": s.location.to_string(),
                       "is_existing_asset": true, "depreciation_method": "Straight Line", "frequency_of_depreciation": 12});
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

/// Three desks for 1,000 over six years: each row divided on its own came to 333.35 for the desk
/// split off (cost 333.33) and 666.65 for the two kept (666.67).
async fn a_split_leaves_each_plan_exact(t: &T, s: &Setup) {
    let desks = t
        .insert(
            "Asset",
            asset(
                s,
                json!({"asset_name": "Desks", "asset_quantity": 3, "purchase_date": "2020-01-01", "gross_purchase_amount": "1000",
                            "total_number_of_depreciations": 6, "start_convention": "Full First Period",
                            "finance_books": [{"finance_book": s.tax.to_string(), "total_number_of_depreciations": 3}]}),
            ),
        )
        .await;
    let desks = uid(&desks);
    t.submit("Asset", desks).await;
    let done = t.k.run_action(&t.admin, "Asset", desks, "split_asset", json!({"split_qty": 1})).await.expect("split");
    let one: Uuid = done["asset"].as_str().unwrap().parse().unwrap();
    let (two, desk) = (t.get("Asset", desks).await, t.get("Asset", one).await);
    assert_eq!((dec(&two["gross_purchase_amount"]), dec(&desk["gross_purchase_amount"])), (n("666.67"), n("333.33")));
    for x in [&two, &desk] {
        assert_eq!(unbooked(x, None), dec(&x["value_after_depreciation"]) - dec(&x["salvage_value"]), "{}: the plan ends at salvage", x["asset_quantity"]);
        assert_eq!(
            unbooked(x, Some(s.tax)),
            dec(&rows(x, "finance_books")[0]["value_after_depreciation"]),
            "{}: and so does the tax book's",
            x["asset_quantity"]
        );
    }
    // The last row of each took the difference; together each date still adds up to the row before.
    assert_eq!(plan(&desk, None).iter().map(|r| r.1).collect::<Vec<_>>(), [n("55.56"), n("55.56"), n("55.56"), n("55.56"), n("55.56"), n("55.53")]);
    assert_eq!(plan(&two, None).iter().map(|r| r.1).collect::<Vec<_>>(), [n("111.11"), n("111.11"), n("111.11"), n("111.11"), n("111.11"), n("111.12")]);
    t.upkeep().await;
    let entries = t.entries(desks).await + t.entries(one).await;
    t.upkeep().await;
    assert_eq!(t.entries(desks).await + t.entries(one).await, entries, "a second upkeep posts nothing");
    for a in [desks, one] {
        let x = t.get("Asset", a).await;
        assert_eq!((x["status"].clone(), dec(&x["value_after_depreciation"])), (json!("Fully Depreciated"), n("0")), "{}", x["asset_quantity"]);
        assert_eq!(dec(&rows(&x, "finance_books")[0]["value_after_depreciation"]), n("0"), "{}: the tax book too", x["asset_quantity"]);
        t.posted_once(a).await;
    }
}

/// A mill of 10,000 from 10 March 2021: monthly by days in the common books, yearly by Half Year in
/// the tax book, whose years end in December.
async fn a_half_year_book_beside_a_common_one(t: &T, s: &Setup) -> Uuid {
    let mill = t
        .insert(
            "Asset",
            asset(
                s,
                json!({"asset_name": "Mill", "purchase_date": "2021-03-10", "gross_purchase_amount": "10000", "total_number_of_depreciations": 60,
                            "frequency_of_depreciation": 1, "start_convention": "Pro-rata by Days",
                            "finance_books": [{"finance_book": s.tax.to_string(), "total_number_of_depreciations": 5, "frequency_of_depreciation": 12,
                                               "start_convention": "Half Year", "depreciation_start_date": "2021-12-31"}]}),
            ),
        )
        .await;
    let tax: Vec<(String, Decimal)> = plan(&mill, Some(s.tax)).into_iter().map(|r| (r.0, r.1)).collect();
    let years: Vec<(String, Decimal)> = [("2021", "1000"), ("2022", "2000"), ("2023", "2000"), ("2024", "2000"), ("2025", "2000"), ("2026", "1000")]
        .iter()
        .map(|(y, a)| (format!("{y}-12-31"), n(a)))
        .collect();
    assert_eq!(tax, years, "half a year first and last");
    let own = plan(&mill, None);
    assert_eq!((own.len(), own[0].0.as_str(), own[60].0.as_str()), (61, "2021-03-31", "2026-03-31"), "by days: the rest of March at the end");
    assert_eq!(own.iter().map(|r| r.1).sum::<Decimal>(), n("10000"));
    let mill = uid(&mill);
    t.submit("Asset", mill).await;
    mill
}

async fn a_convention_must_fit_its_periods(t: &T, s: &Setup) {
    let monthly = asset(
        s,
        json!({"asset_name": "Chair", "purchase_date": "2021-03-10", "gross_purchase_amount": "100", "total_number_of_depreciations": 12,
               "frequency_of_depreciation": 1, "start_convention": "Half Year"}),
    );
    let refused = t.k.insert(&t.admin, "Asset", obj(monthly)).await;
    assert!(matches!(refused, Err(ref e) if validation(e, "start_convention") && validation(e, "whole years")), "{refused:?}");
}

/// The mill is scrapped on the last day of 2023, after the upkeep booked that year in full: the tax
/// book counts half of 2023, not all of it.
async fn leaving_under_half_year_takes_half_the_year(t: &T, s: &Setup, mill: Uuid) {
    t.upkeep().await;
    let before = t.get("Asset", mill).await;
    assert!(plan(&before, Some(s.tax)).iter().any(|r| r.0 == "2023-12-31" && r.1 == n("2000") && r.2), "2023 booked in full");
    t.posted_once(mill).await;
    let scrap = t
        .insert("AssetDisposal", json!({"company": s.company.to_string(), "asset": mill.to_string(), "disposal_type": "Scrap", "disposal_date": "2023-12-31"}))
        .await;
    t.submit("AssetDisposal", uid(&scrap)).await;
    let gone = t.get("Asset", mill).await;
    let tax = plan(&gone, Some(s.tax));
    assert_eq!(booked(&gone, Some(s.tax)), [n("1000"), n("2000"), n("1000")], "{tax:?}");
    assert_eq!(tax.len(), 3, "nothing left to plan: {tax:?}");
    let last = rows(&gone, "book_schedule").into_iter().find(|r| r["disposal"] == json!(scrap["id"])).expect("the disposal's row");
    assert_eq!((last["schedule_date"].as_str(), dec(&last["depreciation_amount"])), (Some("2023-12-31"), n("1000")));
    assert!(plan(&gone, None).iter().all(|r| r.2 && r.0.as_str() <= "2023-12-31"), "the common books stop at the end of December");
    assert_eq!(gone["status"], json!("Scrapped"));
    t.posted_once(mill).await;
    t.upkeep().await;
    t.posted_once(mill).await;
    // Undone: the plan is back as it was, to be booked again.
    t.cancel("AssetDisposal", uid(&scrap)).await;
    let back = t.get("Asset", mill).await;
    let dates = |x: &Doc, b: Option<Uuid>| plan(x, b).into_iter().map(|r| (r.0, r.1)).collect::<Vec<_>>();
    assert_eq!(dates(&back, Some(s.tax)), dates(&before, Some(s.tax)));
    assert_eq!(dates(&back, None), dates(&before, None));
    t.posted_once(mill).await;
    let p: HashMap<String, String> =
        [("company", s.company.to_string()), ("from_date", "2019-01-01".into()), ("to_date", chrono::Utc::now().date_naive().to_string())]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
    let tb = ferrum_domain::reports::run(&t.k, &t.admin, "trial_balance", &p).await.expect("trial balance");
    assert_eq!(tb["meta"]["balanced"], json!(true), "{}", tb["totals"]);
}

fn amounts(doc: &Doc, book: Option<Uuid>) -> Vec<Decimal> {
    plan(doc, book).into_iter().map(|r| r.1).collect()
}

async fn sum_of_years_on_an_asset_and_in_a_book(t: &T, s: &Setup) {
    let syd = |extra: Value| {
        let mut v = asset(
            s,
            json!({"asset_name": "Press", "purchase_date": "2021-01-01", "gross_purchase_amount": "15000", "total_number_of_depreciations": 5,
                                    "start_convention": "Full First Period"}),
        );
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        v
    };
    let own = t.insert("Asset", syd(json!({"depreciation_method": "Sum of Years Digits"}))).await;
    assert_eq!(amounts(&own, None), [n("5000"), n("4000"), n("3000"), n("2000"), n("1000")]);
    let beside = t.insert("Asset", syd(json!({"finance_books": [{"finance_book": s.tax.to_string(), "depreciation_method": "Sum of Years Digits"}]}))).await;
    assert_eq!(amounts(&beside, None), [n("3000"); 5], "straight line in the common books");
    assert_eq!(amounts(&beside, Some(s.tax)), [n("5000"), n("4000"), n("3000"), n("2000"), n("1000")], "sum of years in the tax book");
    let odd =
        t.k.insert(
            &t.admin,
            "Asset",
            obj(syd(json!({"depreciation_method": "Sum of Years Digits", "total_number_of_depreciations": 30, "frequency_of_depreciation": 1}))),
        )
        .await;
    assert!(matches!(odd, Err(ref e) if validation(e, "whole years")), "{odd:?}");
}

/// A truck of 50,000 with a salvage value of 5,000, expected to run 100,000 km.
async fn usage_becomes_depreciation(t: &T, s: &Setup) {
    let truck = t
        .insert(
            "Asset",
            asset(
                s,
                json!({"asset_name": "Truck", "purchase_date": "2025-01-01", "gross_purchase_amount": "50000", "salvage_value": "5000",
                            "depreciation_method": "Units of Production", "total_number_of_depreciations": 60, "frequency_of_depreciation": 1,
                            "total_estimated_units": "100000", "usage_unit": "km"}),
            ),
        )
        .await;
    assert!(plan(&truck, None).is_empty(), "nothing is planned ahead");
    let truck = uid(&truck);
    t.submit("Asset", truck).await;
    let a = &t.admin;
    let first = t.usage(a, truck, "2025-01-31", "2000").await.expect("usage");
    assert_eq!((dec(&first["depreciation_amount"]), first["units_left"].as_str()), (n("900"), Some("98000")));
    t.usage(a, truck, "2025-01-31", "2000").await.expect("the same again");
    assert_eq!(plan(&t.get("Asset", truck).await, None), [("2025-01-31".to_string(), n("900"), false)], "one row for the period");
    t.usage(a, truck, "2025-01-31", "3000").await.expect("corrected");
    let x = t.get("Asset", truck).await;
    assert_eq!(plan(&x, None), [("2025-01-31".to_string(), n("1350"), false)]);
    assert_eq!(dec(&rows(&x, "schedule")[0]["units"]), n("3000"));
    t.upkeep().await;
    assert_eq!(t.entries(truck).await, 1, "booked by exactly one entry");
    let booked_period = t.usage(a, truck, "2025-01-31", "100").await;
    assert!(matches!(booked_period, Err(ref e) if validation(e, "booked up to 2025-01-31")), "{booked_period:?}");
    t.usage(a, truck, "2025-02-28", "500").await.expect("usage");
    t.usage(a, truck, "2025-02-28", "0").await.expect("taken back");
    assert_eq!(plan(&t.get("Asset", truck).await, None).len(), 1, "0 units removes the unbooked row");
    let last = t.usage(a, truck, "2025-03-31", "500000").await.expect("beyond the estimate");
    assert_eq!(dec(&last["depreciation_amount"]), n("43650"), "the rest, to salvage exactly");
    t.upkeep().await;
    let x = t.get("Asset", truck).await;
    assert_eq!((x["status"].clone(), dec(&x["value_after_depreciation"])), (json!("Fully Depreciated"), n("5000")));
    t.posted_once(truck).await;
}

/// A forklift of 10,000 expected to run 1,000 hours, its usage recorded through the gate.
async fn usage_through_the_gate_repeats_safely(t: &T, s: &Setup) -> Uuid {
    let forklift = t
        .insert(
            "Asset",
            asset(
                s,
                json!({"asset_name": "Forklift", "purchase_date": "2025-01-01", "gross_purchase_amount": "10000", "depreciation_method": "Units of Production",
                            "total_number_of_depreciations": 60, "frequency_of_depreciation": 1, "total_estimated_units": "1000", "usage_unit": "hours"}),
            ),
        )
        .await;
    let forklift = uid(&forklift);
    t.submit("Asset", forklift).await;
    let call = |units: &str| {
        Invocation::new("assets.asset.record_usage", json!({"id": forklift.to_string(), "params": {"period_end": "2025-01-31", "units": units}})).key("u1")
    };
    let once = t.k.invoke(&t.admin, call("100")).await.expect("usage");
    assert!(!once.repeated);
    assert_eq!(dec(&once.result["depreciation_amount"]), n("1000"));
    let again = t.k.invoke(&t.admin, call("100")).await.expect("repeat");
    assert!(again.repeated, "the second answer is the first's");
    assert_eq!(plan(&t.get("Asset", forklift).await, None), [("2025-01-31".to_string(), n("1000"), false)]);
    let changed = t.k.invoke(&t.admin, call("150")).await;
    assert!(matches!(changed, Err(Error::Refused { code: "input_changed", .. })), "{changed:?}");
    forklift
}

async fn who_may_record_usage(t: &T, forklift: Uuid, straight_line: Uuid) {
    let user = t.user("user@acme.test", "Asset User").await;
    let refused = t.usage(&user, forklift, "2025-01-31", "100").await;
    assert!(matches!(refused, Err(Error::Forbidden(_))), "an Asset User only reads assets: {refused:?}");
    let future = (chrono::Utc::now().date_naive() + chrono::Duration::days(40)).to_string();
    let later = t.usage(&t.admin, forklift, &future, "100").await;
    assert!(matches!(later, Err(ref e) if validation(e, "today or before")), "{later:?}");
    let not_offered = t.usage(&t.admin, straight_line, "2025-01-31", "100").await;
    assert!(matches!(not_offered, Err(ref e) if validation(e, "not available")), "{not_offered:?}");
}

/// The forklift's value is halved between two usages: the planned row halves and keeps its units.
async fn a_revaluation_between_usages_is_carried(t: &T, s: &Setup, forklift: Uuid) {
    t.upkeep().await;
    t.usage(&t.admin, forklift, "2025-03-31", "200").await.expect("usage");
    let x = t.get("Asset", forklift).await;
    assert_eq!(plan(&x, None).last().map(|r| (r.1, r.2)), Some((n("2000"), false)), "9,000 left over 900 hours");
    let expenses = t.find_one("Account", json!({"account_name": "Administrative Expenses", "company": s.company.to_string()})).await;
    let adjust = t
        .insert(
            "AssetValueAdjustment",
            json!({"asset": forklift.to_string(), "adjustment_date": "2025-02-28", "new_asset_value": "4500", "difference_account": expenses["id"]}),
        )
        .await;
    t.submit("AssetValueAdjustment", uid(&adjust)).await;
    let x = t.get("Asset", forklift).await;
    let row = rows(&x, "schedule").into_iter().last().unwrap();
    assert_eq!((dec(&row["depreciation_amount"]), dec(&row["units"]), row["booked"].clone()), (n("1000"), n("200"), json!(false)));
    t.usage(&t.admin, forklift, "2025-04-30", "10000").await.expect("the rest");
    let x = t.get("Asset", forklift).await;
    assert_eq!(unbooked(&x, None), dec(&x["value_after_depreciation"]), "the plan still ends at salvage");
    t.upkeep().await;
    let x = t.get("Asset", forklift).await;
    assert_eq!((x["status"].clone(), dec(&x["value_after_depreciation"])), (json!("Fully Depreciated"), n("0")));
    t.posted_once(forklift).await;
}

fn years(table: &Doc) -> Vec<(i64, Decimal)> {
    let mut v: Vec<(i64, Decimal)> = rows(table, "rows").iter().map(|r| (r["year"].as_i64().unwrap(), dec(&r["percent"]))).collect();
    v.sort_by_key(|r| r.0);
    v
}

async fn the_seeded_tables_total_100(t: &T) {
    let tables = t.list("DepreciationTable", &json!({})).await;
    assert_eq!(tables.len(), 6, "{tables:?}");
    for table in &tables {
        let full = t.get("DepreciationTable", uid(table)).await;
        assert_eq!(dec(&full["total_percent"]), n("100"), "{}", full["table_name"]);
        let y = years(&full);
        assert!(y.iter().enumerate().all(|(i, r)| r.0 == i as i64 + 1), "{}: {y:?}", full["table_name"]);
    }
    let five = t.find_one("DepreciationTable", json!({"table_name": "US GDS 5-year, half-year"})).await;
    let five = t.get("DepreciationTable", uid(&five)).await;
    assert_eq!(years(&five).len(), 6, "a five-year life in six rows: the half year is in the table");
}

async fn table_id(t: &T, name: &str) -> String {
    t.find_one("DepreciationTable", json!({"table_name": name})).await["id"].as_str().unwrap().to_string()
}

/// A lathe of 10,000: straight line over five years in the common books, the five-year table in the tax book.
async fn a_tax_book_depreciates_by_a_table(t: &T, s: &Setup) {
    let five = table_id(t, "US GDS 5-year, half-year").await;
    let lathe = t
        .insert(
            "Asset",
            asset(
                s,
                json!({"asset_name": "Lathe", "purchase_date": "2019-06-01", "gross_purchase_amount": "10000", "total_number_of_depreciations": 5,
                            "start_convention": "Full First Period",
                            "finance_books": [{"finance_book": s.tax.to_string(), "depreciation_method": "Table", "depreciation_table": five,
                                               "depreciation_start_date": "2019-12-31"}]}),
            ),
        )
        .await;
    assert_eq!(amounts(&lathe, None), [n("2000"); 5]);
    assert_eq!(amounts(&lathe, Some(s.tax)), [n("2000"), n("3200"), n("1920"), n("1152"), n("1152"), n("576")]);
    assert_eq!(rows(&lathe, "finance_books")[0]["total_number_of_depreciations"], json!(6), "the life is the table's");
    let lathe = uid(&lathe);
    t.submit("Asset", lathe).await;
    t.upkeep().await;
    t.upkeep().await;
    let x = t.get("Asset", lathe).await;
    assert_eq!(booked(&x, Some(s.tax)).len(), 6, "every year of the table booked");
    assert_eq!(dec(&rows(&x, "finance_books")[0]["value_after_depreciation"]), n("0"));
    t.posted_once(lathe).await;
}

async fn a_table_changed_later_leaves_a_submitted_asset_alone(t: &T, s: &Setup) {
    let made = t.insert("DepreciationTable", json!({"table_name": "Two years", "rows": [{"year": 1, "percent": "60"}, {"year": 2, "percent": "40"}]})).await;
    let table = uid(&made);
    let by_table = |name: &str| {
        asset(
            s,
            json!({"asset_name": name, "purchase_date": "2024-01-01", "gross_purchase_amount": "1000", "depreciation_method": "Table",
                        "depreciation_table": table.to_string(), "start_convention": "Full First Period"}),
        )
    };
    let kept = t.insert("Asset", by_table("Kept")).await;
    assert_eq!(amounts(&kept, None), [n("600"), n("400")]);
    let kept = uid(&kept);
    t.submit("Asset", kept).await;
    let draft = uid(&t.insert("Asset", by_table("Draft")).await);
    t.k.update(&t.admin, "DepreciationTable", table, obj(json!({"rows": [{"year": 1, "percent": "50"}, {"year": 2, "percent": "50"}]})), None)
        .await
        .expect("the table changes");
    assert_eq!(amounts(&t.get("Asset", kept).await, None), [n("600"), n("400")], "the submitted plan stays");
    let again = t.k.update(&t.admin, "Asset", draft, obj(json!({"asset_name": "Draft again"})), None).await.expect("save the draft");
    assert_eq!(amounts(&again, None), [n("500"), n("500")], "a draft takes the table as it is now");
}

async fn a_table_must_total_100_and_run_without_gaps(t: &T) {
    let short =
        t.k.insert(&t.admin, "DepreciationTable", obj(json!({"table_name": "Short", "rows": [{"year": 1, "percent": "50"}, {"year": 2, "percent": "40"}]})))
            .await;
    assert!(matches!(short, Err(ref e) if validation(e, "total 100")), "{short:?}");
    let gap =
        t.k.insert(&t.admin, "DepreciationTable", obj(json!({"table_name": "Gap", "rows": [{"year": 1, "percent": "50"}, {"year": 3, "percent": "50"}]})))
            .await;
    assert!(matches!(gap, Err(ref e) if validation(e, "without a gap")), "{gap:?}");
}

async fn an_asset_user_cannot_make_a_table(t: &T) {
    let user = t.user("tables@acme.test", "Asset User").await;
    let made = t.k.insert(&user, "DepreciationTable", obj(json!({"table_name": "Mine", "rows": [{"year": 1, "percent": "100"}]}))).await;
    assert!(matches!(made, Err(Error::Forbidden(_))), "{made:?}");
}

async fn a_table_needs_yearly_periods(t: &T, s: &Setup) {
    let five = table_id(t, "US GDS 5-year, half-year").await;
    let monthly =
        t.k.insert(
            &t.admin,
            "Asset",
            obj(asset(
                s,
                json!({"asset_name": "Monthly", "purchase_date": "2024-01-01", "gross_purchase_amount": "1000", "depreciation_method": "Table",
                                "depreciation_table": five, "frequency_of_depreciation": 1}),
            )),
        )
        .await;
    assert!(matches!(monthly, Err(ref e) if validation(e, "frequency_of_depreciation") && validation(e, "yearly periods")), "{monthly:?}");
}
