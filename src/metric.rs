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
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result};
use ferrum_studio::spec::Spec;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
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

/// The metric's value for `ctx`, with its rights to the records the query counts.
pub(crate) async fn value(k: &Kernel, ctx: &Ctx, doc: &Doc) -> Result<Decimal> {
    let spec = spec_of(doc)?;
    let out = ferrum_studio::run::run(k, ctx, &spec).await?;
    let key = out["columns"].as_array().and_then(|c| c.last()).and_then(|c| c["name"].as_str()).unwrap_or_default().to_string();
    let cell = out["rows"].as_array().and_then(|r| r.first()).map(|r| r[&key].clone()).unwrap_or(Value::Null);
    Ok(match cell {
        Value::String(s) => s.parse().unwrap_or_default(),
        Value::Number(n) => n.to_string().parse().unwrap_or_default(),
        _ => Decimal::ZERO,
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
        let Ok(spec) = spec_of(doc) else { return Ok(()) };
        doc.insert("uses".into(), json!(lineage(&spec, |f| reached(hc.ctx, &spec.entity, f))));
        redraft(doc, &fingerprint(&spec), &[]);
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let spec = spec_of(doc)?;
        // The query must make sense to the records and to its author's rights.
        ferrum_studio::query::compile(hc.ctx, &spec).map_err(|e| Error::field("definition", format!("does not run: {e}")))?;
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        let metric = id(doc, "id").ok_or_else(|| Error::internal("a metric without an id"))?;
        let system = elevated(hc.ctx);
        match name {
            "certify" => certify(hc, "Metric", doc, fingerprint(&spec_of(doc)?), json!({})).await.map(Some),
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
