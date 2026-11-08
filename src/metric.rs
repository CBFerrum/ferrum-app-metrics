//! Metrics. A metric is a Studio query of exactly one measure, checked against the records and the
//! author's rights when it is saved. Its lineage is the record and the fields it reads. A Data
//! Steward who does not own it certifies it, which fixes its definition: changing the definition
//! puts it back in draft. Whoever reads it gets its value with their own rights to the records it
//! counts, so two people may read two numbers from one definition. A deprecated metric names what
//! replaces it, and is read no more. Datasets (`dataset`) are governed the same way, by the same
//! helpers here.

use crate::obj;
use async_trait::async_trait;
use chrono::Utc;
use ferrum_calc::formula::Formula;
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result};
use ferrum_studio::spec::{Bucket, Spec, TimeSeries};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use uuid::Uuid;

/// A record's definition, as a Studio query.
pub(crate) fn parse_spec(doc: &Doc) -> Result<Spec> {
    let raw = doc.get("definition").cloned().unwrap_or(Value::Null);
    let raw = match raw {
        Value::String(s) => serde_json::from_str(&s).map_err(|e| Error::field("definition", format!("is not a Studio query: {e}")))?,
        v => v,
    };
    serde_json::from_value(raw).map_err(|e| Error::field("definition", format!("is not a Studio query: {e}")))
}

/// A metric's definition, as a Studio query of one measure.
pub(crate) fn spec_of(doc: &Doc) -> Result<Spec> {
    let spec = parse_spec(doc)?;
    if spec.measures.len() != 1 || !spec.group_by.is_empty() || spec.pivot.is_some() || spec.time_series.is_some() {
        return Err(Error::field("definition", "a metric is one measure, ungrouped"));
    }
    Ok(spec)
}

/// How deep metrics may be composed of others.
const MAX_DEPTH: usize = 8;
/// Most metrics one formula may name.
const MAX_PARTS: usize = 20;

/// What a metric is: a Studio query of one measure, or a formula over other metrics by name
/// (`[revenue] / [orders]`), worked out in exact decimals from each part's value for the reader.
pub(crate) enum Kind {
    Query(Box<Spec>),
    Composed { text: String, formula: Formula, parts: Vec<String> },
}

/// The metrics a formula names, each once, in order of name.
pub(crate) fn parts_of(text: &str) -> Result<Vec<String>> {
    let mut parts = BTreeSet::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let close = rest[open..].find(']').ok_or_else(|| Error::field("formula", "a [ is never closed"))?;
        let name = rest[open + 1..open + close].trim();
        if name.is_empty() {
            return Err(Error::field("formula", "[] names no metric"));
        }
        parts.insert(name.to_string());
        rest = &rest[open + close + 1..];
    }
    if parts.is_empty() {
        return Err(Error::field("formula", "names no metric: write each in brackets, [revenue] / [orders]"));
    }
    if parts.len() > MAX_PARTS {
        return Err(Error::field("formula", format!("names at most {MAX_PARTS} metrics")));
    }
    Ok(parts.into_iter().collect())
}

pub(crate) fn kind_of(doc: &Doc) -> Result<Kind> {
    let formula = s(doc, "formula").map(str::trim).filter(|f| !f.is_empty());
    let defined = !matches!(doc.get("definition"), None | Some(Value::Null)) && doc.get("definition") != Some(&json!(""));
    match (formula, defined) {
        (Some(_), true) => Err(Error::field("formula", "a metric is a query or a formula over other metrics, not both")),
        (Some(text), false) => {
            let parts = parts_of(text)?;
            let formula = Formula::parse(text, &|name| Ok(name.trim().to_string())).map_err(|e| Error::field("formula", e))?;
            Ok(Kind::Composed { text: text.to_string(), formula, parts })
        }
        (None, _) => Ok(Kind::Query(Box::new(spec_of(doc)?))),
    }
}

/// What certification fixes, whatever the metric is.
pub(crate) fn fingerprint_of(kind: &Kind) -> String {
    match kind {
        Kind::Query(spec) => fingerprint(spec),
        Kind::Composed { text, .. } => {
            hex::encode(Sha256::digest(format!("ferrum.metrics.composed/1\n{}", text.split_whitespace().collect::<String>()).as_bytes()))
        }
    }
}

/// The metric named `name` in `ctx`'s workspace, whoever may read it.
pub(crate) async fn named(k: &Kernel, ctx: &Ctx, name: &str) -> Result<Option<Doc>> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let found = k.find_tx(&ferrum_domain::util::sys(ctx), &mut tx, "Metric", &[ferrum_kernel::Filter::eq("metric_name", name)], None, 1).await?;
    tx.commit().await?;
    Ok(found.into_iter().next())
}

/// Every metric that names `name` in its formula, and every one that names those, by name: what a
/// change to it reaches (impact).
pub(crate) async fn used_by(k: &Kernel, ctx: &Ctx, name: &str) -> Result<Vec<String>> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let all: Vec<(String, Option<String>)> =
        sqlx::query_as("select metric_name, formula from e_metric where tenant_id = $1 and coalesce(formula, '') <> '' limit 5000")
            .bind(ctx.tenant)
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    let uses: BTreeMap<String, Vec<String>> = all.into_iter().filter_map(|(n, f)| Some((n, parts_of(f.as_deref()?).ok()?))).collect();
    let (mut out, mut todo) = (BTreeSet::new(), vec![name.to_string()]);
    while let Some(part) = todo.pop() {
        for (m, parts) in &uses {
            if parts.contains(&part) && out.insert(m.clone()) {
                todo.push(m.clone());
            }
        }
    }
    out.remove(name);
    Ok(out.into_iter().collect())
}

/// A formula's parts must be metrics, never itself, even by way of others, none deprecated.
async fn check_parts(k: &Kernel, ctx: &Ctx, me: &str, parts: &[String]) -> Result<()> {
    let mut todo: Vec<(String, Vec<String>)> = parts.iter().map(|p| (p.clone(), vec![me.to_string(), p.clone()])).collect();
    while let Some((part, path)) = todo.pop() {
        if part.eq_ignore_ascii_case(me) {
            return Err(Error::field("formula", format!("a metric is not made of itself, even by way of others: {}", path.join(" → "))));
        }
        if path.len() > MAX_DEPTH + 1 {
            return Err(Error::field("formula", format!("metrics are composed {MAX_DEPTH} deep at most")));
        }
        let Some(doc) = named(k, ctx, &part).await? else {
            return Err(Error::field("formula", format!("there is no metric called {part}")));
        };
        if s(&doc, "status") == Some("Deprecated") {
            return Err(Error::field("formula", format!("{part} is deprecated")));
        }
        if let Some(text) = s(&doc, "formula").filter(|f| !f.trim().is_empty()) {
            for next in parts_of(text)? {
                let mut p = path.clone();
                p.push(next.clone());
                todo.push((next, p));
            }
        }
    }
    Ok(())
}

/// The definition's fingerprint: what certification fixes.
pub(crate) fn fingerprint(spec: &Spec) -> String {
    hex::encode(Sha256::digest(serde_json::to_vec(spec).unwrap_or_default()))
}

/// What a query reads: its record's fields, then each linked record's (or child table's) fields
/// under the record it reaches by which field: "SalesInvoice: grand_total; Customer (by customer):
/// customer_group". `target` names the record a field of the query's record reaches.
pub(crate) fn lineage(spec: &Spec, target: impl Fn(&str) -> Option<String>) -> String {
    let mut paths = BTreeSet::new();
    paths.extend(spec.measures.iter().filter_map(|m| m.field.clone()));
    paths.extend(spec.fields.iter().map(|f| f.field.clone()));
    paths.extend(spec.group_by.iter().map(|g| g.field.clone()));
    paths.extend(spec.pivot.iter().map(|p| p.field.clone()));
    paths.extend(spec.time_series.iter().map(|t| t.field.clone()));
    paths.extend(spec.date_range.iter().map(|d| d.field.clone()));
    if let Ok(filters) = ferrum_kernel::query::parse_filters(&spec.filters) {
        paths.extend(filters.into_iter().map(|f| f.field));
    }
    let (mut own, mut reached) = (BTreeSet::new(), BTreeMap::<String, BTreeSet<String>>::new());
    for p in paths {
        match p.split_once('.') {
            Some((via, rest)) => {
                let record = target(via).map_or_else(|| format!("by {via}"), |t| format!("{t} (by {via})"));
                reached.entry(record).or_default().insert(rest.to_string());
            }
            None => {
                own.insert(p);
            }
        }
    }
    let list = |s: BTreeSet<String>| s.into_iter().collect::<Vec<_>>().join(", ");
    let mut out = if own.is_empty() { spec.entity.clone() } else { format!("{}: {}", spec.entity, list(own)) };
    for (record, fields) in reached {
        out = format!("{out}; {record}: {}", list(fields));
    }
    out
}

/// The record a field of `entity` reaches, by link or as a child table, in `ctx`'s schema.
pub(crate) fn reached(ctx: &Ctx, entity: &str, field: &str) -> Option<String> {
    let f = ctx.schema.entity(entity)?.field(field)?;
    f.to.clone().or_else(|| f.of.clone())
}

/// Whoever certifies and deprecates: a Data Steward, or an administrator.
fn steward(ctx: &Ctx) -> bool {
    ctx.is_admin || ctx.roles.iter().any(|r| r == "Data Steward")
}

/// A certified definition that changed is a draft again, to be certified anew.
pub(crate) fn redraft(doc: &mut Doc, hash: &str, also: &[&str]) {
    if s(doc, "status") == Some("Certified") && s(doc, "certified_hash") != Some(hash) {
        doc.insert("status".into(), json!("Draft"));
        for k in ["certified_by", "certified_at", "certified_hash"].iter().chain(also) {
            doc.insert((*k).into(), Value::Null);
        }
    }
}

/// `certify` on a metric or a dataset: by a steward who does not own it, fixing the definition's
/// fingerprint `hash`, with `more` kept beside it.
pub(crate) async fn certify(hc: &mut HookCtx<'_>, entity: &str, doc: &Doc, hash: String, more: Value) -> Result<Value> {
    let what = entity.to_lowercase();
    if !steward(hc.ctx) {
        return Err(Error::Forbidden(format!("a Data Steward certifies a {what}")));
    }
    if s(doc, "owner").is_some_and(|o| o.eq_ignore_ascii_case(&hc.ctx.email)) {
        return Err(Error::Forbidden(format!("a {what} is certified by someone other than its owner")));
    }
    let record = id(doc, "id").ok_or_else(|| Error::internal("a record without an id"))?;
    let mut values = json!({"status": "Certified", "certified_by": hc.ctx.email, "certified_at": Utc::now().to_rfc3339(), "certified_hash": hash});
    if let (Value::Object(v), Value::Object(m)) = (&mut values, more) {
        v.extend(m);
    }
    hc.kernel.set_values_tx(&elevated(hc.ctx), hc.tx, entity, record, obj(values)).await?;
    Ok(json!({"status": "Certified"}))
}

/// `deprecate` on a metric or a dataset: by a steward, naming what replaces it, never itself.
pub(crate) async fn deprecate(hc: &mut HookCtx<'_>, entity: &str, doc: &Doc, params: &Value) -> Result<Value> {
    if !steward(hc.ctx) {
        return Err(Error::Forbidden(format!("a Data Steward deprecates a {}", entity.to_lowercase())));
    }
    let record = id(doc, "id").ok_or_else(|| Error::internal("a record without an id"))?;
    let replaced: Option<Uuid> = params.get("replaced_by").and_then(Value::as_str).and_then(|v| v.parse().ok());
    if replaced == Some(record) {
        return Err(Error::field("replaced_by", format!("a {} does not replace itself", entity.to_lowercase())));
    }
    let values = json!({"status": "Deprecated", "replaced_by": replaced.map(|r| r.to_string())});
    hc.kernel.set_values_tx(&elevated(hc.ctx), hc.tx, entity, record, obj(values)).await?;
    Ok(json!({"status": "Deprecated"}))
}

/// The metric's value for `ctx`, with its rights to the records the query counts. A composed metric is
/// worked out from each part's value for the same person, so two people may read two numbers from one
/// definition; a part not certified, or a division by zero, leaves it without a value, said so.
pub(crate) async fn value(k: &Kernel, ctx: &Ctx, doc: &Doc) -> Result<Decimal> {
    value_at(k, ctx, doc, 0).await
}

type Valued<'a> = Pin<Box<dyn Future<Output = Result<Decimal>> + Send + 'a>>;

fn value_at<'a>(k: &'a Kernel, ctx: &'a Ctx, doc: &'a Doc, depth: usize) -> Valued<'a> {
    Box::pin(async move {
        match kind_of(doc)? {
            Kind::Query(spec) => query_value(k, ctx, &spec).await,
            Kind::Composed { formula, parts, .. } => {
                let me = s(doc, "metric_name").unwrap_or("this metric").to_string();
                if depth >= MAX_DEPTH {
                    return Err(Error::invalid(format!("{me} is composed more than {MAX_DEPTH} deep")));
                }
                let mut row = serde_json::Map::new();
                for part in &parts {
                    let found = named(k, ctx, part).await?.ok_or_else(|| Error::invalid(format!("{me} has no value: there is no metric called {part}")))?;
                    if s(&found, "status") != Some("Certified") {
                        return Err(Error::invalid(format!("{me} has no value: {part} is not certified")));
                    }
                    let v = value_at(k, ctx, &found, depth + 1).await?;
                    row.insert(part.clone(), json!(v.to_string()));
                }
                formula.eval(&row).ok_or_else(|| Error::invalid(format!("{me} has no value now: a part it divides by is zero, or empty")))
            }
        }
    })
}

/// What a query of one measure counts for `ctx`.
async fn query_value(k: &Kernel, ctx: &Ctx, spec: &Spec) -> Result<Decimal> {
    let out = ferrum_studio::run::run(k, ctx, spec).await?;
    let key = out["columns"].as_array().and_then(|c| c.last()).and_then(|c| c["name"].as_str()).unwrap_or_default().to_string();
    let cell = out["rows"].as_array().and_then(|r| r.first()).map(|r| r[&key].clone()).unwrap_or(Value::Null);
    Ok(match cell {
        Value::String(s) => s.parse().unwrap_or_default(),
        Value::Number(n) => n.to_string().parse().unwrap_or_default(),
        _ => Decimal::ZERO,
    })
}

/// One period of a metric's series. `partial`: the period is not over, so its value is so far.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Point {
    pub start: String,
    pub value: Option<String>,
    pub partial: bool,
}

/// The first day of the period `back` periods before the one holding `today`.
pub(crate) fn period_start(bucket: Bucket, today: chrono::NaiveDate, back: u32) -> Option<chrono::NaiveDate> {
    use chrono::{Datelike, Months, NaiveDate};
    Some(match bucket {
        Bucket::Day => today - chrono::Duration::days(i64::from(back)),
        Bucket::Week => today - chrono::Duration::days(i64::from(today.weekday().num_days_from_monday()) + 7 * i64::from(back)),
        Bucket::Month => NaiveDate::from_ymd_opt(today.year(), today.month(), 1)?.checked_sub_months(Months::new(back))?,
        Bucket::Quarter => NaiveDate::from_ymd_opt(today.year(), (today.month0() / 3) * 3 + 1, 1)?.checked_sub_months(Months::new(3 * back))?,
        Bucket::Year => NaiveDate::from_ymd_opt(today.year() - i32::try_from(back).ok()?, 1, 1)?,
        Bucket::FiscalQuarter | Bucket::FiscalYear => return None,
    })
}

type Series<'a> = Pin<Box<dyn Future<Output = Result<BTreeMap<String, Option<Decimal>>>> + Send + 'a>>;

/// A metric's value for each of the last `periods` periods, the current one marked partial: by its
/// `series_by` date field for a query, period by period from its parts for a composed metric.
pub(crate) async fn series(k: &Kernel, ctx: &Ctx, doc: &Doc, bucket: Bucket, periods: u32) -> Result<Vec<Point>> {
    if !(1..=120).contains(&periods) {
        return Err(Error::field("periods", "is 1 to 120"));
    }
    let today = Utc::now().date_naive();
    let from = period_start(bucket, today, periods - 1).ok_or_else(|| Error::field("bucket", "a series goes by day, week, month, quarter or year"))?;
    let current = period_start(bucket, today, 0).unwrap_or(today).to_string();
    let values = series_at(k, ctx, doc, bucket, from, today, 0).await?;
    Ok(values.into_iter().map(|(start, v)| Point { partial: start >= current, start, value: v.map(|d| d.normalize().to_string()) }).collect())
}

fn series_at<'a>(k: &'a Kernel, ctx: &'a Ctx, doc: &'a Doc, bucket: Bucket, from: chrono::NaiveDate, to: chrono::NaiveDate, depth: usize) -> Series<'a> {
    Box::pin(async move {
        let me = s(doc, "metric_name").unwrap_or("this metric").to_string();
        match kind_of(doc)? {
            Kind::Query(mut spec) => {
                let field = s(doc, "series_by")
                    .filter(|f| !f.trim().is_empty())
                    .ok_or_else(|| Error::field("series_by", format!("{me}: say which date field a series of it goes by")))?;
                spec.time_series =
                    Some(TimeSeries { field: field.trim().to_string(), bucket, from: Some(from.to_string()), to: Some(to.to_string()), fill_zero: true });
                let out = ferrum_studio::run::run(k, ctx, &spec).await?;
                let columns = out["columns"].as_array().cloned().unwrap_or_default();
                let key = columns.last().and_then(|c| c["name"].as_str()).unwrap_or_default().to_string();
                // The period's start is under the column that goes by a bucket.
                let period = columns.iter().find(|c| c.get("bucket").is_some()).and_then(|c| c["name"].as_str()).unwrap_or("bucket").to_string();
                Ok(out["rows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| {
                        let start = r[&period].as_str()?.chars().take(10).collect::<String>();
                        let v = match &r[&key] {
                            Value::String(s) => s.parse().ok(),
                            Value::Number(n) => n.to_string().parse().ok(),
                            _ => Some(Decimal::ZERO),
                        };
                        Some((start, v))
                    })
                    .collect())
            }
            Kind::Composed { formula, parts, .. } => {
                if depth >= MAX_DEPTH {
                    return Err(Error::invalid(format!("{me} is composed more than {MAX_DEPTH} deep")));
                }
                let mut each: Vec<(String, BTreeMap<String, Option<Decimal>>)> = Vec::new();
                for part in &parts {
                    let found = named(k, ctx, part).await?.ok_or_else(|| Error::invalid(format!("{me} has no series: there is no metric called {part}")))?;
                    if s(&found, "status") != Some("Certified") {
                        return Err(Error::invalid(format!("{me} has no series: {part} is not certified")));
                    }
                    each.push((part.clone(), series_at(k, ctx, &found, bucket, from, to, depth + 1).await?));
                }
                let starts: BTreeSet<String> = each.iter().flat_map(|(_, s)| s.keys().cloned()).collect();
                Ok(starts
                    .into_iter()
                    .map(|start| {
                        let mut row = serde_json::Map::new();
                        for (part, values) in &each {
                            if let Some(Some(v)) = values.get(&start) {
                                row.insert(part.clone(), json!(v.to_string()));
                            }
                        }
                        let v = formula.eval(&row);
                        (start, v)
                    })
                    .collect())
            }
        }
    })
}

pub struct MetricHook;

#[async_trait]
impl DocHook for MetricHook {
    fn name(&self) -> &'static str {
        "metrics.metric"
    }

    fn fills(&self) -> &'static [&'static str] {
        &["owner", "uses", "status", "certified_by", "certified_at", "certified_hash"]
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if s(doc, "owner").is_none() && !hc.ctx.email.is_empty() {
            doc.insert("owner".into(), json!(hc.ctx.email.to_lowercase()));
        }
        let Ok(kind) = kind_of(doc) else { return Ok(()) };
        let uses = match &kind {
            Kind::Query(spec) => lineage(spec, |f| reached(hc.ctx, &spec.entity, f)),
            Kind::Composed { parts, .. } => format!("Composed of: {}", parts.join(", ")),
        };
        doc.insert("uses".into(), json!(uses));
        redraft(doc, &fingerprint_of(&kind), &[]);
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        match kind_of(doc)? {
            // The query must make sense to the records and to its author's rights.
            Kind::Query(spec) => {
                ferrum_studio::query::compile(hc.ctx, &spec).map_err(|e| Error::field("definition", format!("does not run: {e}")))?;
            }
            Kind::Composed { parts, .. } => check_parts(hc.kernel, hc.ctx, s(doc, "metric_name").unwrap_or_default(), &parts).await?,
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        let metric = id(doc, "id").ok_or_else(|| Error::internal("a metric without an id"))?;
        let system = elevated(hc.ctx);
        match name {
            "certify" => {
                let kind = kind_of(doc)?;
                // A composed metric is certified after its parts: what it means is what they mean.
                if let Kind::Composed { parts, .. } = &kind {
                    for part in parts {
                        let found = named(hc.kernel, hc.ctx, part).await?;
                        if found.as_ref().and_then(|d| s(d, "status")) != Some("Certified") {
                            return Err(Error::invalid(format!("{part} is certified first: what this means is what its parts mean")));
                        }
                    }
                }
                certify(hc, "Metric", doc, fingerprint_of(&kind), json!({})).await.map(Some)
            }
            "deprecate" => deprecate(hc, "Metric", doc, params).await.map(Some),
            "snapshot" => {
                let v = value(hc.kernel, hc.ctx, doc).await?;
                hc.kernel
                    .insert_tx(
                        &system,
                        hc.tx,
                        "MetricSnapshot",
                        obj(json!({"metric": metric.to_string(), "value": ferrum_domain::util::money(v), "read_by": hc.ctx.email, "read_at": Utc::now().to_rfc3339(),
                                   "certified_hash": s(doc, "certified_hash")})),
                    )
                    .await?;
                Ok(Some(json!({"value": ferrum_domain::util::money(v)})))
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_metric_is_one_measure_and_its_lineage() {
        let doc = |v: Value| v.as_object().cloned().unwrap();
        let ok = doc(json!({"definition": {"entity": "Customer", "filters": [["customer_type", "eq", "Company"]], "measures": [{"fn": "count"}]}}));
        let spec = spec_of(&ok).unwrap();
        assert_eq!(lineage(&spec, |_| None), "Customer: customer_type");
        let summed = doc(json!({"definition": {"entity": "SalesInvoice", "filters": {"status": "Paid"}, "measures": [{"fn": "sum", "field": "grand_total"}]}}));
        assert_eq!(lineage(&spec_of(&summed).unwrap(), |_| None), "SalesInvoice: grand_total, status");
        let pair = doc(json!({"definition": {"entity": "Customer", "filters": [["customer_group", "Retail"]], "measures": [{"fn": "count"}]}}));
        assert_eq!(lineage(&spec_of(&pair).unwrap(), |_| None), "Customer: customer_group");
        // Linked records' fields, under the record each link reaches.
        let joined = parse_spec(&doc(json!({"definition": {"entity": "SalesInvoice", "filters": [["customer.customer_group", "eq", "Retail"]],
            "group_by": [{"field": "customer.territory"}, {"field": "posting_date"}], "measures": [{"fn": "sum", "field": "grand_total"}]}})))
        .unwrap();
        let target = |f: &str| (f == "customer").then(|| "Customer".to_string());
        assert_eq!(lineage(&joined, target), "SalesInvoice: grand_total, posting_date; Customer (by customer): customer_group, territory");
        let two = doc(json!({"definition": {"entity": "Customer", "measures": [{"fn": "count"}, {"fn": "count"}]}}));
        assert!(spec_of(&two).is_err());
        let grouped = doc(json!({"definition": {"entity": "Customer", "group_by": [{"field": "customer_type"}], "measures": [{"fn": "count"}]}}));
        assert!(spec_of(&grouped).is_err());
        let pivoted = doc(json!({"definition": {"entity": "Customer", "pivot": {"field": "customer_type"}, "measures": [{"fn": "count"}]}}));
        assert!(spec_of(&pivoted).is_err());
        let series = doc(json!({"definition": {"entity": "Customer", "time_series": {"field": "creation", "bucket": "month"}, "measures": [{"fn": "count"}]}}));
        assert!(spec_of(&series).is_err());
        assert_ne!(fingerprint(&spec), fingerprint(&spec_of(&doc(json!({"definition": {"entity": "Customer", "measures": [{"fn": "count"}]}}))).unwrap()));
    }
}
