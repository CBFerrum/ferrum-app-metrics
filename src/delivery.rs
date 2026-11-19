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
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::time::Instant;
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

/// What one subscription's delivery comes to, worked out before anything is written.
enum Outcome {
    /// The follower's value: kept as theirs and told.
    Read { user: Uuid, metric: Doc, value: Decimal },
    /// The metric is deprecated since: told once, naming what replaces it, and followed no more.
    Deprecated { user: Uuid, title: String },
    /// Why it could not be delivered, kept on the subscription.
    Failed(Error),
}

/// One subscription's delivery worked out for its follower, reading only: each read in a short transaction of its own, and
/// the value (an external metric's from Metrics' engine) with no transaction open.
async fn outcome(k: &Kernel, ctx: &Ctx, metric_id: Uuid, follower: &str) -> Outcome {
    let found = async {
        let metric = k.get(ctx, "Metric", metric_id, false).await?;
        let mut tx = k.db.tenant_tx(ctx.tenant).await?;
        let user: Option<Uuid> = sqlx::query_scalar("select id from app_user where tenant_id = $1 and is_active and lower(email::text) = lower($2)")
            .bind(ctx.tenant)
            .bind(follower)
            .fetch_optional(&mut *tx)
            .await?;
        tx.commit().await?;
        let user = user.ok_or_else(|| Error::invalid(format!("{follower} is not an active person here")))?;
        Ok::<_, Error>((metric, user))
    };
    let (metric, user) = match found.await {
        Ok(found) => found,
        Err(e) => return Outcome::Failed(e),
    };
    let name = s(&metric, "metric_name").unwrap_or_default().to_string();
    match s(&metric, "status") {
        Some("Certified") => {}
        Some("Deprecated") => {
            let instead = match id(&metric, "replaced_by") {
                Some(r) => k.get(ctx, "Metric", r, false).await.ok().and_then(|m| s(&m, "metric_name").map(str::to_string)),
                None => None,
            };
            let title = match instead {
                Some(n) => format!("{name} is deprecated: follow {n} instead"),
                None => format!("{name} is deprecated"),
            };
            return Outcome::Deprecated { user, title };
        }
        _ => return Outcome::Failed(Error::invalid(format!("{name} is not certified now: it is delivered again once it is"))),
    }
    let read = async {
        let reader = k.load_ctx(ctx.tenant, user, None, "metrics.digest".into()).await?;
        value(k, &reader, &metric).await
    };
    match read.await {
        Ok(value) => Outcome::Read { user, metric, value },
        Err(e) => Outcome::Failed(e),
    }
}

/// The subscription, locked for this runner, when it is still as the round read it (`seen`, when it was last sent):
/// another runner that delivered it since, or holds it now, leaves nothing for this one to do.
async fn claim(tx: &mut Tx, tenant: Uuid, subscription: Uuid, seen: Option<NaiveDate>) -> Result<bool> {
    let held: Option<Uuid> = sqlx::query_scalar(
        "select id from e_metric_subscription where tenant_id = $1 and id = $2 and active and last_sent_on is not distinct from $3 for update skip locked",
    )
    .bind(tenant)
    .bind(subscription)
    .bind(seen)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(held.is_some())
}

/// What `outcome` writes, in the claiming transaction: the follower's value kept and the notice, and what to keep on
/// the subscription.
async fn write(k: &Kernel, ctx: &Ctx, tx: &mut Tx, metric_id: Uuid, follower: &str, today: NaiveDate, outcome: Outcome) -> Result<Value> {
    match outcome {
        Outcome::Read { user, metric, value: v } => {
            let reader = k.load_ctx(ctx.tenant, user, None, "metrics.digest".into()).await?;
            // Kept as the follower's own value, as one they record themselves is.
            k.insert_tx(
                &elevated(&reader),
                tx,
                "MetricSnapshot",
                obj(json!({"metric": metric_id.to_string(), "value": money(v), "read_by": follower, "read_at": Utc::now().to_rfc3339(),
                           "certified_hash": s(&metric, "certified_hash"), "definition": crate::metric::meaning(&metric)})),
            )
            .await?;
            let name = s(&metric, "metric_name").unwrap_or_default();
            let unit = s(&metric, "unit").map(|u| format!(" {u}")).unwrap_or_default();
            let title = format!("{name}: {}{unit}", v.normalize());
            notify::send_kind(tx, ctx.tenant, user, "metrics.digest", "inapp", &title, s(&metric, "description"), Some("Metric"), Some(metric_id)).await?;
            Ok(json!({"last_sent_on": today.to_string(), "last_value": money(v), "last_error": null}))
        }
        Outcome::Deprecated { user, title } => {
            notify::send_kind(tx, ctx.tenant, user, "metrics.digest", "inapp", &title, None, Some("Metric"), Some(metric_id)).await?;
            Ok(json!({"last_sent_on": today.to_string(), "active": false, "last_error": null}))
        }
        Outcome::Failed(e) => Ok(json!({"last_sent_on": today.to_string(), "last_error": said(&e)})),
    }
}

/// A subscription due, as one round read it: its id, metric, follower and when it was last sent.
#[doc(hidden)]
pub type Due = (Uuid, Uuid, Option<String>, Option<NaiveDate>);

/// The subscriptions of `tenant` due on the workspace's day, at most a round's.
#[doc(hidden)]
pub async fn due(k: &Kernel, tenant: Uuid) -> Result<Vec<Due>> {
    let schema = k.registry.schema(tenant).await?;
    let today = Ctx::system(tenant, schema).today();
    let mut tx = k.db.tenant_tx(tenant).await?;
    let due: Vec<Due> = sqlx::query_as(
        "select id, metric, subscriber::text, last_sent_on from e_metric_subscription where tenant_id = $1 and active and metric is not null \
         and (last_sent_on is null or (frequency = 'Daily' and last_sent_on < $2) or (frequency = 'Weekly' and last_sent_on <= $2 - 7) \
              or (frequency = 'Monthly' and last_sent_on <= ($2 - interval '1 month')::date)) order by id limit $3",
    )
    .bind(tenant)
    .bind(today)
    .bind(PER_ROUND)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(due)
}

/// `due` delivered, each in a transaction of its own, so one that cannot be leaves the others delivered and keeps
/// why. How many were delivered.
#[doc(hidden)]
pub async fn deliver_due(k: &Kernel, tenant: Uuid, due: Vec<Due>, deadline: Instant) -> Result<usize> {
    let schema = k.registry.schema(tenant).await?;
    let ctx = Ctx::system(tenant, schema);
    // The workspace's day, not UTC's.
    let today = ctx.today();
    let mut n = 0;
    for (subscription, metric, follower, seen) in due {
        if Instant::now() >= deadline {
            break;
        }
        let follower = follower.unwrap_or_default();
        // Worked out first, with no transaction open; written only by the runner that claims it.
        let done = outcome(k, &ctx, metric, &follower).await;
        let mut tx = k.db.tenant_tx(tenant).await?;
        if !claim(&mut tx, tenant, subscription, seen).await? {
            tx.rollback().await?;
            continue;
        }
        let kept = match write(k, &ctx, &mut tx, metric, &follower, today, done).await {
            Ok(kept) => kept,
            Err(e) => {
                tx.rollback().await?;
                tx = k.db.tenant_tx(tenant).await?;
                if !claim(&mut tx, tenant, subscription, seen).await? {
                    tx.rollback().await?;
                    continue;
                }
                json!({"last_sent_on": today.to_string(), "last_error": said(&e)})
            }
        };
        k.set_values_tx(&ctx, &mut tx, "MetricSubscription", subscription, obj(kept)).await?;
        tx.commit().await?;
        n += 1;
    }
    Ok(n)
}

/// A workspace's round: each subscription due on its day delivered, within `deadline`. How many were.
pub(crate) async fn deliver(k: &Kernel, tenant: Uuid, deadline: Instant) -> Result<usize> {
    let rows = due(k, tenant).await?;
    deliver_due(k, tenant, rows, deadline).await
}
