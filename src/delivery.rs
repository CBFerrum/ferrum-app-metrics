//! Metrics followed. A person follows a certified metric, daily, weekly or monthly; upkeep reads it
//! for them with their own rights on the workspace's day it is due, keeps that as a value, and tells
//! them the way they chose for `metrics.digest`. A metric deprecated since is said so once, naming
//! what replaces it, and is no longer delivered; one back in draft waits until it is certified
//! again.

use crate::metric::value;
use crate::obj;
use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use ferrum_domain::util::{elevated, id, money, s};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result, Tx, notify};
use serde_json::{Value, json};
use uuid::Uuid;

/// Subscriptions delivered in a round, at most.
const PER_ROUND: i64 = 200;

pub struct SubscriptionHook;

#[async_trait]
impl DocHook for SubscriptionHook {
    fn name(&self) -> &'static str {
        "metrics.subscription"
    }

    fn fills(&self) -> &'static [&'static str] {
        &["subscriber"]
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        // A person follows for themselves.
        if !hc.ctx.is_system && !hc.ctx.email.is_empty() {
            doc.insert("subscriber".into(), json!(hc.ctx.email.to_lowercase()));
        }
        Ok(())
    }

    async fn validate_change(&self, hc: &mut HookCtx<'_>, old: Option<&Doc>, doc: &Doc) -> Result<()> {
        let metric = id(doc, "metric").ok_or_else(|| Error::field("metric", "is required"))?;
        if old.is_none_or(|o| id(o, "metric") != Some(metric)) {
            let status: Option<Option<String>> = sqlx::query_scalar("select status from e_metric where tenant_id = $1 and id = $2")
                .bind(hc.ctx.tenant)
                .bind(metric)
                .fetch_optional(&mut **hc.tx)
                .await?;
            if status.flatten().as_deref() != Some("Certified") {
                return Err(Error::field("metric", "only a certified metric is followed"));
            }
        }
        let twice: Option<i32> = sqlx::query_scalar(
            "select 1 from e_metric_subscription where tenant_id = $1 and metric = $2 and lower(subscriber::text) = lower($3) and active and id is distinct from $4 limit 1",
        )
        .bind(hc.ctx.tenant)
        .bind(metric)
        .bind(s(doc, "subscriber").unwrap_or_default())
        .bind(id(doc, "id"))
        .fetch_optional(&mut **hc.tx)
        .await?;
        if twice.is_some() && doc.get("active").and_then(Value::as_bool).unwrap_or(true) {
            return Err(Error::field("metric", "you follow it already"));
        }
        Ok(())
    }
}

/// An error as a person reads it, a refused field's message included.
fn said(e: &Error) -> String {
    match e {
        Error::Validation(fields) => fields.iter().map(|f| format!("{}: {}", f.field, f.message)).collect::<Vec<_>>().join("; "),
        other => other.to_string(),
    }
}

/// One subscription's delivery: what to keep on it.
async fn deliver_one(k: &Kernel, ctx: &Ctx, tx: &mut Tx, metric_id: Uuid, follower: &str, today: NaiveDate) -> Result<Value> {
    let metric = k.get_tx(ctx, tx, "Metric", metric_id, false).await?;
    let name = s(&metric, "metric_name").unwrap_or_default().to_string();
    let user: Option<Uuid> = sqlx::query_scalar("select id from app_user where tenant_id = $1 and is_active and lower(email::text) = lower($2)")
        .bind(ctx.tenant)
        .bind(follower)
        .fetch_optional(&mut **tx)
        .await?;
    let user = user.ok_or_else(|| Error::invalid(format!("{follower} is not an active person here")))?;
    match s(&metric, "status") {
        Some("Certified") => {}
        Some("Deprecated") => {
            let instead = match id(&metric, "replaced_by") {
                Some(r) => k.get_tx(ctx, tx, "Metric", r, false).await.ok().and_then(|m| s(&m, "metric_name").map(str::to_string)),
                None => None,
            };
            let title = match instead {
                Some(n) => format!("{name} is deprecated: follow {n} instead"),
                None => format!("{name} is deprecated"),
            };
            notify::send_kind(tx, ctx.tenant, user, "metrics.digest", "inapp", &title, None, Some("Metric"), Some(metric_id)).await?;
            return Ok(json!({"last_sent_on": today.to_string(), "active": false, "last_error": null}));
        }
        _ => return Err(Error::invalid(format!("{name} is not certified now: it is delivered again once it is"))),
    }
    let reader = k.load_ctx(ctx.tenant, user, None, "metrics.digest".into()).await?;
    let v = value(k, &reader, &metric).await?;
    // Kept as the follower's own value, as one they record themselves is.
    k.insert_tx(
        &elevated(&reader),
        tx,
        "MetricSnapshot",
        obj(json!({"metric": metric_id.to_string(), "value": money(v), "read_by": follower, "read_at": Utc::now().to_rfc3339(),
                   "certified_hash": s(&metric, "certified_hash"), "definition": crate::metric::meaning(&metric)})),
    )
    .await?;
    let unit = s(&metric, "unit").map(|u| format!(" {u}")).unwrap_or_default();
    let title = format!("{name}: {}{unit}", v.normalize());
    notify::send_kind(tx, ctx.tenant, user, "metrics.digest", "inapp", &title, s(&metric, "description"), Some("Metric"), Some(metric_id)).await?;
    Ok(json!({"last_sent_on": today.to_string(), "last_value": money(v), "last_error": null}))
}

/// A workspace's round: each subscription due on its day delivered, each in a transaction of its
/// own, so one that cannot be leaves the others delivered and keeps why. How many were due.
pub(crate) async fn deliver(k: &Kernel, tenant: Uuid) -> Result<usize> {
    let schema = k.registry.schema(tenant).await?;
    if !schema.packages.iter().any(|p| p == "metrics") {
        return Ok(0);
    }
    let ctx = Ctx::system(tenant, schema);
    // The workspace's day, not UTC's.
    let today = ctx.today();
    let mut tx = k.db.tenant_tx(tenant).await?;
    let due: Vec<(Uuid, Uuid, Option<String>)> = sqlx::query_as(
        "select id, metric, subscriber::text from e_metric_subscription where tenant_id = $1 and active and metric is not null \
         and (last_sent_on is null or (frequency = 'Daily' and last_sent_on < $2) or (frequency = 'Weekly' and last_sent_on <= $2 - 7) \
              or (frequency = 'Monthly' and last_sent_on <= ($2 - interval '1 month')::date)) order by id limit $3",
    )
    .bind(tenant)
    .bind(today)
    .bind(PER_ROUND)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let n = due.len();
    for (subscription, metric, follower) in due {
        let mut tx = k.db.tenant_tx(tenant).await?;
        let outcome = deliver_one(k, &ctx, &mut tx, metric, follower.as_deref().unwrap_or_default(), today).await;
        let kept = match outcome {
            Ok(v) => v,
            Err(e) => {
                tx.rollback().await?;
                tx = k.db.tenant_tx(tenant).await?;
                json!({"last_sent_on": today.to_string(), "last_error": said(&e)})
            }
        };
        k.set_values_tx(&ctx, &mut tx, "MetricSubscription", subscription, obj(kept)).await?;
        tx.commit().await?;
    }
    Ok(n)
}
