//! Metrics' engine for external sources, as Metrics asks it (the service is `engine/` in this repository;
//! Ferrum's Logic Platform: FLC-SEM-005, -006, -007, -010).
//!
//! A metric whose numbers live outside the records names a Wren semantic model over the engine's datasets
//! and one query over it (`external`). Each dataset the model reads is described here, by a Data Steward, as
//! an External Source: which rows and columns each reader sees, and how fresh its data must be. The engine
//! is asked with what this reader may see of each dataset, never more, and says when each was last written,
//! so that a value read from stale data is said to be stale. Nothing here or there changes anything: the
//! engine is read as a database is, within a time limit, so a retried save or delivery reads again and does
//! nothing twice.
//!
//! The engine's address and token are the operator's: `FERRUM_METRICS_ENGINE_URL` (a private address, plain
//! http allowed) and `FERRUM_METRICS_ENGINE_TOKEN`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ferrum_domain::util::{s, sys};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, Filter, HookCtx, Kernel, Result};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// How long Metrics waits for the engine; the engine stops a query sooner, by its own limit.
const ENGINE_SECS: u64 = 15;
const MOST_MODELS: usize = 200;
const MOST_SQL: usize = 20_000;

/// A metric read from external datasets: its Wren semantic model, its one query, and the datasets its
/// models read.
#[derive(Debug, Clone)]
pub(crate) struct External {
    pub model: Value,
    pub sql: String,
    pub datasets: Vec<String>,
}

impl External {
    /// What it reads, for its lineage.
    pub(crate) fn lineage(&self) -> String {
        let models: Vec<&str> = self.model["models"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str()).collect();
        format!("External: {} through Wren (models {})", self.datasets.join(", "), models.join(", "))
    }
}

/// A dataset a value was read from whose data is older than its source allows (or of no known age).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Stale {
    pub source: String,
    pub written_at: Option<String>,
    pub fresh_within_minutes: i64,
}

impl Stale {
    pub(crate) fn json(&self) -> Value {
        json!({"source": self.source, "written_at": self.written_at, "fresh_within_minutes": self.fresh_within_minutes})
    }
}

/// A dataset's name as the engine lists it: lower-case letters, digits and `_`.
fn plain(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_') && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A JSON field, whether kept as JSON or as its text; nothing when empty.
fn json_field(doc: &Doc, field: &str) -> Result<Option<Value>> {
    match doc.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(t)) if t.trim().is_empty() => Ok(None),
        Some(Value::String(t)) => serde_json::from_str(t).map(Some).map_err(|e| Error::field(field, format!("is not JSON: {e}"))),
        Some(v) => Ok(Some(v.clone())),
    }
}

/// A metric's `external` field, if it has one: `{"model": <a Wren semantic model>, "sql": "select …"}`.
pub(crate) fn external_of(doc: &Doc) -> Result<Option<External>> {
    let Some(raw) = json_field(doc, "external")? else { return Ok(None) };
    let obj = raw.as_object().ok_or_else(|| Error::field("external", "is {\"model\": a Wren semantic model, \"sql\": one query over it}"))?;
    if let Some(other) = obj.keys().find(|k| !["model", "sql"].contains(&k.as_str())) {
        return Err(Error::field("external", format!("{other} is not part of an external metric: model, sql")));
    }
    let model = obj.get("model").filter(|m| m.is_object()).cloned().ok_or_else(|| Error::field("external", "names no semantic model (model)"))?;
    let sql = obj.get("sql").and_then(Value::as_str).map(str::trim).filter(|q| !q.is_empty()).ok_or_else(|| Error::field("external", "has no query (sql)"))?;
    if sql.len() > MOST_SQL {
        return Err(Error::field("external", format!("its query is at most {MOST_SQL} characters")));
    }
    let models = model["models"]
        .as_array()
        .filter(|m| !m.is_empty() && m.len() <= MOST_MODELS)
        .ok_or_else(|| Error::field("external", format!("its semantic model has 1 to {MOST_MODELS} models")))?;
    let mut datasets = BTreeSet::new();
    for m in models {
        let table = m["tableReference"]["table"]
            .as_str()
            .filter(|t| plain(t))
            .ok_or_else(|| Error::field("external", "each model names its dataset (tableReference.table), in lower case"))?;
        datasets.insert(table.to_string());
    }
    Ok(Some(External { model, sql: sql.to_string(), datasets: datasets.into_iter().collect() }))
}

/// The External Source describing `dataset` in this workspace.
async fn source(k: &Kernel, ctx: &Ctx, dataset: &str) -> Result<Doc> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let found = k.find_tx(&sys(ctx), &mut tx, "ExternalSource", &[Filter::eq("source_name", dataset)], None, 1).await?;
    tx.commit().await?;
    found
        .into_iter()
        .next()
        .ok_or_else(|| Error::field("external", format!("{dataset} is not an external source here: a Data Steward describes it under External Sources first")))
}

/// A source's rules for the rows each reader sees: one rule or a list of them, each `{column, by: email}`
/// or `{column, by: roles, map: {role: [values]}}`.
fn row_rules(doc: &Doc) -> Result<Vec<Value>> {
    let name = s(doc, "source_name").unwrap_or("this source").to_string();
    let bad = |why: &str| Error::field("rows_by", format!("{name}: {why}"));
    let rules = match json_field(doc, "rows_by")? {
        None => return Ok(vec![]),
        Some(Value::Array(a)) => a,
        Some(v @ Value::Object(_)) => vec![v],
        Some(_) => return Err(bad("is a rule, or a list of rules")),
    };
    for r in &rules {
        let o = r.as_object().ok_or_else(|| bad("each rule is an object"))?;
        if let Some(other) = o.keys().find(|k| !["column", "by", "map"].contains(&k.as_str())) {
            return Err(bad(&format!("{other} is not part of a rule: column, by, map")));
        }
        if o.get("column").and_then(Value::as_str).is_none_or(|c| c.trim().is_empty()) {
            return Err(bad("each rule names a column"));
        }
        match o.get("by").and_then(Value::as_str) {
            Some("email") => {}
            Some("roles") => {
                let map = o.get("map").and_then(Value::as_object).ok_or_else(|| bad("a rule by roles maps each role to its values (map)"))?;
                if !map.values().all(|v| v.as_array().is_some_and(|a| a.iter().all(Value::is_string))) {
                    return Err(bad("each role maps to a list of values"));
                }
            }
            _ => return Err(bad("by is email or roles")),
        }
    }
    Ok(rules)
}

/// A source's columns only some roles see: `{column: [role, …]}`.
fn column_rules(doc: &Doc) -> Result<BTreeMap<String, Vec<String>>> {
    let bad = || Error::field("hidden_columns", "is {column: [role, …]}");
    let Some(v) = json_field(doc, "hidden_columns")? else { return Ok(BTreeMap::new()) };
    v.as_object()
        .ok_or_else(bad)?
        .iter()
        .map(|(c, roles)| {
            let roles = roles.as_array().filter(|a| a.iter().all(Value::is_string)).ok_or_else(bad)?;
            Ok((c.clone(), roles.iter().filter_map(Value::as_str).map(str::to_string).collect()))
        })
        .collect()
}

/// What `ctx` may see of each dataset, and how fresh each must be, from its External Source.
async fn allowed(k: &Kernel, ctx: &Ctx, ext: &External) -> Result<(Vec<Value>, BTreeMap<String, i64>)> {
    let (mut out, mut fresh) = (Vec::new(), BTreeMap::new());
    for dataset in &ext.datasets {
        let doc = source(k, ctx, dataset).await?;
        fresh.insert(dataset.clone(), doc.get("fresh_within_minutes").and_then(Value::as_i64).unwrap_or(0));
        let everything = ctx.is_admin || ctx.is_system;
        let rows: Vec<Value> = if everything {
            vec![]
        } else {
            row_rules(&doc)?
                .iter()
                .map(|r| match r["by"].as_str() {
                    Some("email") => json!({"column": r["column"], "values": [ctx.email.to_lowercase()], "any_case": true}),
                    _ => {
                        let map = r["map"].as_object().cloned().unwrap_or_default();
                        let values: BTreeSet<&str> = ctx
                            .roles
                            .iter()
                            .filter_map(|role| map.get(role))
                            .flat_map(|v| v.as_array().into_iter().flatten())
                            .filter_map(Value::as_str)
                            .collect();
                        json!({"column": r["column"], "values": values, "any_case": false})
                    }
                })
                .collect()
        };
        let hide: Vec<String> = if everything {
            vec![]
        } else {
            column_rules(&doc)?.into_iter().filter(|(_, roles)| !roles.iter().any(|r| ctx.roles.contains(r))).map(|(c, _)| c).collect()
        };
        out.push(json!({"name": dataset, "rows": rows, "hide": hide}));
    }
    Ok((out, fresh))
}

/// The workspace's slug, as the engine lists datasets by (its id serves too).
async fn workspace(k: &Kernel, ctx: &Ctx) -> String {
    let slug = async {
        let mut tx = k.db.system_tenant_tx(ctx.tenant).await?;
        let slug: String = sqlx::query_scalar("select slug from tenant where id = $1").bind(ctx.tenant).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok::<_, Error>(slug)
    };
    slug.await.unwrap_or_else(|_| ctx.tenant.to_string())
}

/// The engine's answer to `path` with `body`, or why there is none.
async fn ask(k: &Kernel, path: &str, body: &Value) -> Result<Value> {
    let base = std::env::var("FERRUM_METRICS_ENGINE_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .ok_or_else(|| Error::Unavailable("Metrics' engine for external sources is not set up on this server (FERRUM_METRICS_ENGINE_URL)".into()))?;
    let token = std::env::var("FERRUM_METRICS_ENGINE_TOKEN").unwrap_or_default();
    let address = format!("{}/{path}", base.trim_end_matches('/'));
    // The engine is the operator's, on a private network: its address may be private, and plain http.
    let (client, url) = k.vetted_client(&address, true, Duration::from_secs(ENGINE_SECS), "ferrum-metrics").await?;
    let r = client.post(url).bearer_auth(token).json(body).send().await.map_err(|e| Error::Unavailable(format!("Metrics' engine cannot be reached: {e}")))?;
    let status = r.status().as_u16();
    let answer: Value = r.json().await.map_err(|e| Error::Unavailable(format!("Metrics' engine answered with something that is not JSON: {e}")))?;
    let said = answer["error"].as_str().unwrap_or("it gave no reason").to_string();
    match status {
        200..=299 => Ok(answer),
        401 => Err(Error::internal("Metrics' engine refused its token: FERRUM_METRICS_ENGINE_TOKEN differs from the engine's")),
        400 | 404 | 413 | 422 => Err(Error::invalid(said)),
        403 => Err(Error::Forbidden(said)),
        _ => Err(Error::Unavailable(format!("Metrics' engine: {said}"))),
    }
}

async fn body(k: &Kernel, ctx: &Ctx, ext: &External, datasets: Vec<Value>) -> Value {
    json!({"workspace": workspace(k, ctx).await, "tenant": ctx.tenant.to_string(), "manifest": ext.model, "sql": ext.sql, "datasets": datasets})
}

/// An external metric checked as it is saved: each dataset described here, and its query planned through
/// its model on the saver's own view of the datasets, answering with one number.
pub(crate) async fn check(k: &Kernel, ctx: &Ctx, ext: &External) -> Result<()> {
    let (datasets, _) = allowed(k, ctx, ext).await?;
    let request = body(k, ctx, ext, datasets).await;
    let checked = ask(k, "check", &request).await.map_err(|e| Error::field("external", format!("does not run: {}", said(&e))))?;
    let columns = checked["columns"].as_array().cloned().unwrap_or_default();
    if columns.len() != 1 || columns[0]["numeric"] != json!(true) {
        return Err(Error::field("external", "its query answers with one number: one column, of a numeric type"));
    }
    Ok(())
}

/// An external metric's value for `ctx`, read with what `ctx` may see, and the datasets it read that are
/// older than their sources allow. An empty sum is zero, as it is for a query of the records.
pub(crate) async fn read(k: &Kernel, ctx: &Ctx, me: &str, ext: &External) -> Result<(Decimal, Vec<Stale>)> {
    let (datasets, fresh) = allowed(k, ctx, ext).await?;
    let request = body(k, ctx, ext, datasets).await;
    let no_value = |e: Error| match e {
        Error::Unavailable(m) => Error::Unavailable(format!("{me} has no value now: {m}")),
        Error::Forbidden(m) => Error::Forbidden(format!("{me} has no value for you: {m}")),
        other => Error::invalid(format!("{me} has no value: {}", said(&other))),
    };
    let answer = ask(k, "query", &request).await.map_err(no_value)?;
    let rows = answer["rows"].as_array().cloned().unwrap_or_default();
    if rows.len() != 1 || rows[0].as_array().map(Vec::len) != Some(1) {
        return Err(Error::invalid(format!("{me} has no value: its query answered with other than one number")));
    }
    let value = match &rows[0][0] {
        Value::Null => Decimal::ZERO,
        Value::String(t) => t
            .parse::<Decimal>()
            .or_else(|_| Decimal::from_scientific(t))
            .map_err(|_| Error::invalid(format!("{me} has no value: its query answered {t}, not a number")))?,
        other => return Err(Error::invalid(format!("{me} has no value: its query answered {other}, not a number"))),
    };
    Ok((value, stale_of(&answer["datasets"], &fresh, Utc::now())))
}

/// The datasets read whose data is older than their sources allow, or of no known age.
pub(crate) fn stale_of(datasets: &Value, fresh: &BTreeMap<String, i64>, now: DateTime<Utc>) -> Vec<Stale> {
    datasets
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let name = d["name"].as_str()?;
            let within = *fresh.get(name)?;
            if within <= 0 {
                return None;
            }
            let written = d["written_at"].as_str();
            let old = match written.and_then(|w| DateTime::parse_from_rfc3339(w).ok()) {
                Some(t) => now.signed_duration_since(t.with_timezone(&Utc)) > chrono::Duration::minutes(within),
                None => true,
            };
            old.then(|| Stale { source: name.to_string(), written_at: written.map(str::to_string), fresh_within_minutes: within })
        })
        .collect()
}

/// An error as a person reads it, a refused field's message included.
fn said(e: &Error) -> String {
    match e {
        Error::Validation(fields) => fields.iter().map(|f| format!("{}: {}", f.field, f.message)).collect::<Vec<_>>().join("; "),
        Error::Invalid(m) | Error::Forbidden(m) | Error::Unavailable(m) => m.clone(),
        other => other.to_string(),
    }
}

/// An External Source is saved only with rules that say something: a plain dataset name, rules of a known
/// shape.
pub struct SourceHook;

#[async_trait]
impl DocHook for SourceHook {
    fn name(&self) -> &'static str {
        "metrics.external_source"
    }

    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if !s(doc, "source_name").is_some_and(plain) {
            return Err(Error::field("source_name", "is the dataset's name on the engine: lower-case letters, digits and _"));
        }
        row_rules(doc)?;
        column_rules(doc)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_when_older_than_allowed_or_of_no_known_age() {
        let now: DateTime<Utc> = "2026-11-08T12:00:00Z".parse().unwrap();
        let fresh = BTreeMap::from([("a".to_string(), 60), ("b".to_string(), 60), ("c".to_string(), 0), ("d".to_string(), 60)]);
        let read = json!([
            {"name": "a", "written_at": "2026-11-08T11:30:00Z"},
            {"name": "b", "written_at": "2026-11-08T10:59:00Z"},
            {"name": "c", "written_at": "2020-01-01T00:00:00Z"},
            {"name": "d", "written_at": null}
        ]);
        let stale: Vec<String> = stale_of(&read, &fresh, now).into_iter().map(|s| s.source).collect();
        assert_eq!(stale, ["b", "d"], "a is within the hour, c is never stale, d is of no known age");
    }

    #[test]
    fn an_external_metric_names_its_model_query_and_datasets() {
        let doc = |v: Value| v.as_object().cloned().unwrap();
        let ok = doc(
            json!({"external": {"model": {"models": [{"name": "orders", "tableReference": {"table": "orders_lake"}}]}, "sql": "select sum(total) from orders"}}),
        );
        let e = external_of(&ok).unwrap().unwrap();
        assert_eq!(
            (e.datasets.as_slice(), e.lineage()),
            (["orders_lake".to_string()].as_slice(), "External: orders_lake through Wren (models orders)".to_string())
        );
        for bad in [
            json!({"external": {"model": {"models": []}, "sql": "select 1"}}),
            json!({"external": {"model": {"models": [{"name": "o", "tableReference": {"table": "Orders"}}]}, "sql": "select 1"}}),
            json!({"external": {"model": {"models": [{"name": "o", "tableReference": {"table": "o_lake"}}]}}}),
            json!({"external": {"model": {"models": [{"name": "o", "tableReference": {"table": "o_lake"}}]}, "sql": "select 1", "engine": "x"}}),
        ] {
            assert!(external_of(&doc(bad.clone())).is_err(), "{bad}");
        }
        assert!(external_of(&doc(json!({"external": null}))).unwrap().is_none());
    }
}
