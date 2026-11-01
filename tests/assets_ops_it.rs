//! Lockout, return to service and maintenance from telemetry, against a real PostgreSQL
//! (`FERRUM_ASSETS_OPS_TEST_DATABASE_URL`; reset first).

use chrono::Utc;
use ferrum_kernel::query::parse_filters;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings, ListQuery, SiteRequest};
use ferrum_meta::Catalog;
use ferrum_ops::commands::{self, NewCommand};
use ferrum_ops::contract::telemetry::{Class, Measurement, Quality};
use ferrum_ops::contract::{Lifecycle, sign::Enroll};
use ferrum_ops::{devices, identity, telemetry};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use uuid::Uuid;

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn uid(d: &Doc) -> Uuid {
    d["id"].as_str().unwrap().parse().unwrap()
}

async fn post(k: &Arc<Kernel>, ctx: &Ctx, path: &str, body: Value) -> Result<Value, Error> {
    let site = k.hooks.site("assets_ops").unwrap().clone();
    let req = SiteRequest { tenant: ctx.tenant, user: Some(ctx.clone()), method: "POST".into(), path: path.into(), query: Map::new(), headers: Map::new(), body: body.to_string().into_bytes(), client_ip: "::1".into() };
    site.handle(k, req).await.map(|r| serde_json::from_slice(&r.body).unwrap())
}

#[test]
fn equipment_goes_out_of_service_and_comes_back_on_purpose() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(32 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

fn key(seed: Uuid) -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(seed.as_bytes());
    ed25519_dalek::SigningKey::from_bytes(&bytes).verifying_key().to_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

async fn scenario() {
    let url = std::env::var("FERRUM_ASSETS_OPS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_assets_ops_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 10, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().unwrap();
    packages.extend(ferrum_app_erp::packages().unwrap());
    packages.extend(ferrum_ops::packages().unwrap());
    packages.extend(ferrum_app_assets::packages().unwrap());
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_erp::register(&mut hooks);
    ferrum_ops::register(&mut hooks);
    ferrum_app_assets::register(&mut hooks);
    let catalog = Catalog::from_packages(packages).unwrap();
    assert_eq!(catalog.links_out_of_reach("assets_ops"), Vec::<String>::new(), "whoever fills in a link can look up what it points at");
    let k = Kernel::boot(db, catalog, hooks, KernelSettings::default(), None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.unwrap();
    ferrum_app_erp::maintenance(&k).await.unwrap();
    let tenant = k
        .create_tenant(NewTenant { slug: "mill".into(), name: "Mill Co".into(), admin_email: "admin@mill.test".into(), admin_password: "Correct-Horse-9".into(), admin_name: None, packages: vec!["assets_ops".into()], settings: Some(json!({"fiscal_year_start_month": 4})) })
        .await
        .expect("tenant");
    let session = k.login("mill", "admin@mill.test", "Correct-Horse-9").await.unwrap();
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.unwrap();
    let person = async |email: &str, roles: &[&str]| {
        let id = k.create_user(&admin, email, "Someone", Some("Correct-Horse-9"), &roles.iter().map(|r| r.to_string()).collect::<Vec<_>>(), false).await.unwrap();
        k.load_ctx(tenant, id, None, "it".into()).await.unwrap()
    };
    let site = uid(&k.insert(&admin, "Site", obj(json!({"site_name": "Mill", "site_code": "mill"}))).await.unwrap());
    let device = async |code: &str, kind: &str, ops: Value| {
        let id = uid(&k.insert(&admin, "Device", obj(json!({"device_name": code, "device_code": code, "kind": kind, "site": site, "operations": ops}))).await.unwrap());
        devices::set_lifecycle(&k, &admin, id, Lifecycle::Claimed, "ours").await.unwrap();
        let (code, _) = identity::new_enrollment(&k, &admin, id).await.unwrap();
        identity::enroll(&k, tenant, &Enroll { code, public_key: key(id) }).await.unwrap();
        devices::set_lifecycle(&k, &admin, id, Lifecycle::Commissioned, "checked").await.unwrap();
        devices::set_lifecycle(&k, &admin, id, Lifecycle::Active, "in service").await.unwrap();
        id
    };
    device("GW", "Gateway", json!([])).await;
    let press = device("PRESS", "Machine", json!([{"operation": "machine.start_job"}])).await;
    let sends = async || {
        let mut tx = k.db.tenant_tx(tenant).await.unwrap();
        let r = commands::issue(&mut tx, &admin, NewCommand { target: press, operation: "machine.start_job".into(), schema_version: 1, payload: json!({"job": 1}), expires_in_secs: 60, ..Default::default() }).await;
        if r.is_ok() {
            tx.commit().await.unwrap();
        }
        r
    };
    assert!(sends().await.is_ok());

    let tech = person("tech@mill.test", &["Maintenance Technician"]).await;
    let authority = person("auth@mill.test", &["Return to Service Authority"]).await;

    // ───── out of service: nothing can be sent ─────
    let request = uid(&k.insert(&tech, "MaintenanceRequest", obj(json!({"title": "Bearing noise", "device": press}))).await.unwrap());
    let locked = post(&k, &tech, "/lockouts", json!({"device": press, "reason": "Bearing replacement", "permit": "P-41", "request": request})).await.expect("locked");
    let lockout: Uuid = locked["id"].as_str().unwrap().parse().unwrap();
    assert!(sends().await.is_err(), "while it is locked out, no work reaches it");
    assert!(post(&k, &tech, "/lockouts", json!({"device": press, "reason": "Again"})).await.is_err(), "one at a time here");

    // ───── finishing the work restarts nothing ─────
    assert!(post(&k, &authority, &format!("/lockouts/{lockout}/return"), json!({"note": "Guards back on"})).await.is_err(), "the work is not recorded as done");
    assert!(post(&k, &tech, &format!("/requests/{request}/repaired"), json!({"work_done": " "})).await.is_err());
    post(&k, &tech, &format!("/requests/{request}/repaired"), json!({"work_done": "Replaced the bearing", "root_cause": "Lubrication"})).await.expect("repaired");
    assert!(sends().await.is_err(), "repaired is not returned to service");
    assert!(matches!(post(&k, &tech, &format!("/lockouts/{lockout}/return"), json!({"note": "done"})).await, Err(Error::Forbidden(_))), "a technician does not return it");
    assert!(post(&k, &authority, &format!("/lockouts/{lockout}/return"), json!({"note": " "})).await.is_err(), "say what you checked");
    post(&k, &authority, &format!("/lockouts/{lockout}/return"), json!({"note": "Guards on, test run clean"})).await.expect("returned");
    assert!(sends().await.is_ok(), "now it can run");

    // ───── meters and conditions ─────
    let meter = uid(&k.insert(&admin, "AssetMeter", obj(json!({"meter_name": "Press hours", "device": press, "metric": "run.hours", "service_every": 100, "next_service_at": 100}))).await.unwrap());
    let rule = uid(&k.insert(&admin, "ConditionRule", obj(json!({"rule_name": "Vibration high", "device": press, "metric": "vibration.rms", "above": 7, "sustained_minutes": 2}))).await.unwrap());
    let boot = Uuid::now_v7();
    let mut seq = 0;
    let mut feed = async |metric: &str, value: f64, minutes_ago: i64| {
        seq += 1;
        telemetry::ensure_partitions(&k, Utc::now()).await.unwrap();
        let mut tx = k.db.tenant_tx(tenant).await.unwrap();
        let m = Measurement { device_id: press, metric: metric.into(), value, unit: None, source_time: Utc::now() - chrono::Duration::minutes(minutes_ago), boot, seq, quality: Quality::Good, class: Class::Sample };
        telemetry::ingest(&mut tx, tenant, site, vec![m], Utc::now()).await.unwrap();
        tx.commit().await.unwrap();
    };
    feed("run.hours", 99.0, 0).await;
    ferrum_app_assets::ops::watch(&k).await.unwrap();
    let count = async |source: &str| k.list(&admin, "MaintenanceRequest", &ListQuery { filters: parse_filters(&json!({"source": source})).unwrap(), ..Default::default() }).await.unwrap().rows.len();
    assert_eq!(count("Meter").await, 0, "not due yet");
    feed("run.hours", 101.0, 0).await;
    ferrum_app_assets::ops::watch(&k).await.unwrap();
    ferrum_app_assets::ops::watch(&k).await.unwrap();
    assert_eq!(count("Meter").await, 1, "due, once");
    assert_eq!(k.get(&admin, "AssetMeter", meter, false).await.unwrap()["next_service_at"], json!("200"), "and the next service moves on");

    feed("vibration.rms", 9.0, 0).await;
    ferrum_app_assets::ops::watch(&k).await.unwrap();
    assert_eq!(count("Condition").await, 0, "one high minute is not sustained");
    feed("vibration.rms", 9.0, 1).await;
    feed("vibration.rms", 9.0, 2).await;
    ferrum_app_assets::ops::watch(&k).await.unwrap();
    ferrum_app_assets::ops::watch(&k).await.unwrap();
    assert_eq!(count("Condition").await, 1, "sustained, once, while it is open");
    let _ = rule;
}
