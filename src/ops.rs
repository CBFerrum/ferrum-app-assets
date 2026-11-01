//! Maintenance of equipment that is also a device. A lockout takes it out of service, so nothing
//! can be sent to it, and the only way back is a return to service: finishing the work never
//! restarts anything. Meters and conditions open requests from telemetry, and missing data opens
//! nothing. A lockout here is a record and a software interlock on dispatch. Physical isolation is
//! the site's own procedure, and this does not replace it.

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ferrum_domain::util::{id, s, sys};
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, Site, SiteRequest, SiteResponse, Tx, audit};
use ferrum_ops::contract::Lifecycle;
use ferrum_ops::devices;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

fn may(ctx: &Ctx, roles: &[&str]) -> Result<()> {
    if ctx.is_admin || roles.iter().any(|r| ctx.has_role(r)) {
        return Ok(());
    }
    Err(Error::Forbidden(format!("this takes one of: {}", roles.join(", "))))
}

fn values(v: Value) -> Doc {
    v.as_object().cloned().unwrap_or_default()
}

#[derive(Deserialize)]
struct Lock {
    device: Uuid,
    reason: String,
    #[serde(default)]
    permit: Option<String>,
    #[serde(default)]
    request: Option<Uuid>,
}

async fn lock(k: &Kernel, ctx: &Ctx, tx: &mut Tx, l: &Lock) -> Result<Value> {
    may(ctx, &["Maintenance Technician", "Maintenance Planner"])?;
    if l.reason.trim().is_empty() {
        return Err(Error::field("reason", "say why"));
    }
    let sctx = sys(ctx);
    let open: Option<Uuid> = sqlx::query_scalar("select id from e_equipment_lockout where tenant_id = $1 and device = $2 and status = 'Locked' limit 1").bind(ctx.tenant).bind(l.device).fetch_optional(&mut **tx).await?;
    if open.is_some() {
        return Err(Error::invalid("this equipment is already locked out"));
    }
    devices::set_lifecycle_tx(k, &sctx, tx, l.device, Lifecycle::Maintenance, &l.reason).await?;
    let doc = json!({"device": l.device, "reason": l.reason, "permit": l.permit, "request": l.request, "locked_by": ctx.email});
    let made = k.insert_tx(&ferrum_domain::util::elevated(ctx), tx, "EquipmentLockout", values(doc)).await?;
    Ok(json!({"id": id(&made, "id")}))
}

/// The handover back to operations. Someone with this authority, who did not lock it out, says it
/// is fit to run, after the work was recorded as done. A second lockout on the device stays in force.
async fn return_to_service(k: &Kernel, ctx: &Ctx, tx: &mut Tx, lockout: Uuid, note: &str) -> Result<()> {
    may(ctx, &["Return to Service Authority"])?;
    if note.trim().is_empty() {
        return Err(Error::field("note", "say what you checked"));
    }
    let sctx = sys(ctx);
    let doc = k.get_tx(&sctx, tx, "EquipmentLockout", lockout, true).await?;
    if s(&doc, "status") != Some("Locked") {
        return Err(Error::invalid("this lockout has ended"));
    }
    if s(&doc, "locked_by") == Some(ctx.email.as_str()) {
        return Err(Error::Forbidden("someone other than who locked it out has to return it".into()));
    }
    if let Some(request) = id(&doc, "request") {
        let r = k.get_tx(&sctx, tx, "MaintenanceRequest", request, true).await?;
        if !matches!(s(&r, "status"), Some("Repaired" | "Closed" | "Rejected")) {
            return Err(Error::invalid("the work on this equipment has not been recorded as done"));
        }
    }
    k.set_values_tx(&sctx, tx, "EquipmentLockout", lockout, values(json!({"status": "Returned", "returned_by": ctx.email, "returned_at": Utc::now().to_rfc3339(), "return_note": note}))).await?;
    let device = id(&doc, "device").ok_or_else(|| Error::internal("a lockout without a device"))?;
    let other: Option<Uuid> = sqlx::query_scalar("select id from e_equipment_lockout where tenant_id = $1 and device = $2 and status = 'Locked' limit 1").bind(ctx.tenant).bind(device).fetch_optional(&mut **tx).await?;
    if other.is_none() {
        devices::set_lifecycle_tx(k, &sctx, tx, device, Lifecycle::Active, note).await?;
    }
    audit::record(tx, ctx, "EquipmentLockout", lockout, "maintenance.returned", None, json!({"note": note})).await
}

#[derive(Deserialize)]
struct Repaired {
    work_done: String,
    #[serde(default)]
    failure_code: Option<Uuid>,
    #[serde(default)]
    root_cause: Option<String>,
}

/// The work is done. This is not a restart.
async fn repaired(k: &Kernel, ctx: &Ctx, tx: &mut Tx, request: Uuid, r: &Repaired) -> Result<()> {
    may(ctx, &["Maintenance Technician", "Maintenance Planner"])?;
    if r.work_done.trim().is_empty() {
        return Err(Error::field("work_done", "say what was done"));
    }
    k.set_values_tx(&sys(ctx), tx, "MaintenanceRequest", request, values(json!({"status": "Repaired", "work_done": r.work_done, "failure_code": r.failure_code, "root_cause": r.root_cause}))).await
}

async fn open_request(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: Value) -> Result<()> {
    k.insert_tx(&ferrum_domain::util::elevated(ctx), tx, "MaintenanceRequest", values(doc)).await?;
    Ok(())
}

/// Meters follow their devices' newest good reading, and service falls due by use.
async fn meters(k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<()> {
    #[derive(sqlx::FromRow)]
    struct Meter {
        id: Uuid,
        meter_name: String,
        asset: Option<Uuid>,
        device: Uuid,
        service_every: Decimal,
        next_service_at: Decimal,
        value: Option<f64>,
        at: Option<chrono::DateTime<Utc>>,
    }
    let rows: Vec<Meter> = sqlx::query_as(
        "select m.id, m.meter_name, m.asset, m.device, m.service_every, m.next_service_at, r.last_value as value, r.last_time as at from e_asset_meter m \
         left join lateral (select last_value, last_time from op_rollup_minute x where x.tenant_id = m.tenant_id and x.device_id = m.device and x.metric = m.metric \
           and x.last_time is not null order by x.minute desc limit 1) r on true where m.tenant_id = $1",
    )
    .bind(ctx.tenant)
    .fetch_all(&mut **tx)
    .await?;
    for m in rows {
        let (Some(value), Some(at)) = (m.value.and_then(|v| Decimal::try_from(v).ok()), m.at) else { continue };
        let mut next = m.next_service_at;
        if value >= next {
            let doc = json!({"title": format!("{} reached {}", m.meter_name, value.normalize()), "asset": m.asset, "device": m.device, "source": "Meter", "meter": m.id, "evidence": {"reading": value, "due_at": next, "read_at": at}});
            open_request(k, ctx, tx, doc).await?;
            while value >= next {
                next += m.service_every;
            }
        }
        k.set_values_tx(&sys(ctx), tx, "AssetMeter", m.id, values(json!({"reading": value, "read_at": at.to_rfc3339(), "next_service_at": next}))).await?;
    }
    Ok(())
}

/// A rule fires only on minutes that have data, every one of the window: a silent sensor is a
/// different problem, and not a reading.
async fn conditions(k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<()> {
    #[derive(sqlx::FromRow)]
    struct Rule {
        id: Uuid,
        rule_name: String,
        asset: Option<Uuid>,
        device: Uuid,
        metric: String,
        above: Option<f64>,
        below: Option<f64>,
        sustained_minutes: Option<i64>,
    }
    let rules: Vec<Rule> = sqlx::query_as(
        "select r.id, r.rule_name, r.asset, r.device, r.metric, r.above::float8, r.below::float8, r.sustained_minutes from e_condition_rule r \
         where r.tenant_id = $1 and r.active and not exists (select 1 from e_maintenance_request q where q.tenant_id = r.tenant_id and q.rule = r.id and q.status in ('Open', 'In Progress'))",
    )
    .bind(ctx.tenant)
    .fetch_all(&mut **tx)
    .await?;
    for r in rules {
        let window = r.sustained_minutes.unwrap_or(5).max(1);
        let since = Utc::now() - Duration::minutes(window);
        let (minutes, peak, low): (i64, Option<f64>, Option<f64>) = sqlx::query_as(
            "select count(*) filter (where samples > 0), max(max_value), min(min_value) from op_rollup_minute where tenant_id = $1 and device_id = $2 and metric = $3 and minute >= date_trunc('minute', $4::timestamptz)",
        )
        .bind(ctx.tenant)
        .bind(r.device)
        .bind(&r.metric)
        .bind(since)
        .fetch_one(&mut **tx)
        .await?;
        let lows_above = |limit: f64| low.is_some_and(|v| v > limit);
        let peaks_below = |limit: f64| peak.is_some_and(|v| v < limit);
        if minutes >= window && (r.above.is_some_and(lows_above) || r.below.is_some_and(peaks_below)) {
            let doc = json!({"title": r.rule_name, "asset": r.asset, "device": r.device, "source": "Condition", "rule": r.id, "evidence": {"metric": r.metric, "minutes": minutes, "low": low, "peak": peak}});
            open_request(k, ctx, tx, doc).await?;
        }
    }
    Ok(())
}

/// Periodic: each workspace with this connector.
pub async fn watch(k: &Kernel) -> Result<()> {
    let tenants: Vec<Uuid> = {
        let mut tx = k.db.system_tx().await?;
        let exists: Option<String> = sqlx::query_scalar("select to_regclass('e_asset_meter')::text").fetch_one(&mut *tx).await?;
        let rows = if exists.is_some() {
            sqlx::query_scalar("select distinct tenant_id from e_asset_meter union select distinct tenant_id from e_condition_rule").fetch_all(&mut *tx).await?
        } else {
            vec![]
        };
        tx.commit().await?;
        rows
    };
    for tenant in tenants {
        let ctx = Ctx::system(tenant, k.registry.schema(tenant).await?);
        let mut tx = k.db.tenant_tx(tenant).await?;
        meters(k, &ctx, &mut tx).await?;
        conditions(k, &ctx, &mut tx).await?;
        tx.commit().await?;
    }
    Ok(())
}

/// Endpoints under `/api/app/assets_ops/`: `POST /lockouts`, `/lockouts/{id}/return`,
/// `/requests/{id}/repaired`.
pub struct MaintenanceSite;

#[async_trait]
impl Site for MaintenanceSite {
    async fn handle(&self, k: &Kernel, req: SiteRequest) -> Result<SiteResponse> {
        let Some(ctx) = req.user.clone() else { return Err(Error::Unauthorized("sign in required".into())) };
        let path = req.path.trim_matches('/').to_string();
        let segments: Vec<&str> = path.split('/').collect();
        let uuid = |v: &str| v.parse::<Uuid>().map_err(|_| Error::NotFound("record".into()));
        let body = || serde_json::from_slice::<Value>(&req.body).map_err(|e| Error::invalid(e.to_string()));
        let mut tx = k.db.tenant_tx(ctx.tenant).await?;
        let answer = match (req.method.as_str(), segments.as_slice()) {
            ("POST", ["lockouts"]) => {
                let l: Lock = serde_json::from_value(body()?).map_err(|e| Error::invalid(e.to_string()))?;
                lock(k, &ctx, &mut tx, &l).await?
            }
            ("POST", ["lockouts", id, "return"]) => {
                let note = body()?.get("note").and_then(Value::as_str).unwrap_or_default().to_string();
                return_to_service(k, &ctx, &mut tx, uuid(id)?, &note).await?;
                json!({"returned": true})
            }
            ("POST", ["requests", id, "repaired"]) => {
                let r: Repaired = serde_json::from_value(body()?).map_err(|e| Error::invalid(e.to_string()))?;
                repaired(k, &ctx, &mut tx, uuid(id)?, &r).await?;
                json!({"repaired": true})
            }
            _ => return Err(Error::NotFound("route".into())),
        };
        tx.commit().await?;
        Ok(SiteResponse::json(200, &answer))
    }
}
