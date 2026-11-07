//! Datasets. A dataset is a table defined once as a Studio query: its columns (record fields, or
//! groupings and measures, spread across the values a pivot names or beside the period before),
//! filters and sort, reaching linked records and child tables by `link.field` when it needs them. Like a metric, it is checked against the records and its
//! author's rights when saved, and its lineage names every record and field it reads. A Data
//! Steward who does not own it certifies it, which keeps its columns then (what a warehouse loads);
//! changing it puts it back in draft. Each reader gets its rows with their own rights, in the app
//! or loaded into a warehouse (`warehouse`).

use crate::metric::{certify, deprecate, fingerprint, lineage, parse_spec, reached, redraft};
use async_trait::async_trait;
use ferrum_domain::util::s;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result};
use ferrum_studio::spec::{MAX_LIMIT, Spec};
use serde_json::{Value, json};

/// A dataset's definition: a Studio query whose columns are the same whatever its rows. A pivot
/// names its values, so they are its columns; a comparison adds each measure's previous period.
pub(crate) fn dataset_spec(doc: &Doc) -> Result<Spec> {
    let spec = parse_spec(doc)?;
    if spec.pivot.as_ref().is_some_and(|p| p.values.is_empty()) {
        return Err(Error::field("definition", "a dataset keeps the same columns whatever its rows: its pivot names its values (pivot.values)"));
    }
    if spec.fields.is_empty() && spec.group_by.is_empty() && spec.measures.is_empty() {
        return Err(Error::field("definition", "a dataset names its columns: record fields, or groupings and measures"));
    }
    Ok(spec)
}

/// A dataset's rows for `ctx`, with their rights: (columns, rows, whether they were cut short). With
/// no limit of its own, it gives as many rows as Studio gives at most.
pub(crate) async fn rows(k: &Kernel, ctx: &Ctx, doc: &Doc) -> Result<(Vec<Value>, Vec<Value>, bool)> {
    let mut spec = dataset_spec(doc)?;
    spec.limit = Some(spec.limit.unwrap_or(MAX_LIMIT));
    let out = ferrum_studio::run::run(k, ctx, &spec).await?;
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    Ok((list(&out["columns"]), list(&out["rows"]), out["meta"]["truncated"].as_bool().unwrap_or(false)))
}

/// Columns as a dataset keeps them: "name (Label): type", one a line.
pub(crate) fn columns_text(columns: &[Value]) -> String {
    columns
        .iter()
        .map(|c| {
            let (name, label, ty) = (c["name"].as_str().unwrap_or_default(), c["label"].as_str().unwrap_or_default(), c["type"].as_str().unwrap_or("data"));
            if label.is_empty() || label == name { format!("{name}: {ty}") } else { format!("{name} ({label}): {ty}") }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct DatasetHook;

#[async_trait]
impl DocHook for DatasetHook {
    fn name(&self) -> &'static str {
        "metrics.dataset"
    }

    fn fills(&self) -> &'static [&'static str] {
        &["owner", "uses", "columns", "status", "certified_by", "certified_at", "certified_hash"]
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if s(doc, "owner").is_none() && !hc.ctx.email.is_empty() {
            doc.insert("owner".into(), json!(hc.ctx.email.to_lowercase()));
        }
        let Ok(spec) = dataset_spec(doc) else { return Ok(()) };
        doc.insert("uses".into(), json!(lineage(&spec, |f| reached(hc.ctx, &spec.entity, f))));
        redraft(doc, &fingerprint(&spec), &["columns"]);
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let spec = dataset_spec(doc)?;
        // The query must make sense to the records and to its author's rights.
        ferrum_studio::query::compile(hc.ctx, &spec).map_err(|e| Error::field("definition", format!("does not run: {e}")))?;
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        match name {
            "certify" => {
                let hash = fingerprint(&dataset_spec(doc)?);
                // Its columns as the steward reads them: what it promises a warehouse.
                let (columns, _, _) = rows(hc.kernel, hc.ctx, doc).await?;
                certify(hc, "Dataset", doc, hash, json!({"columns": columns_text(&columns)})).await.map(Some)
            }
            "deprecate" => deprecate(hc, "Dataset", doc, params).await.map(Some),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dataset_keeps_its_columns() {
        let doc = |v: Value| v.as_object().cloned().unwrap();
        let joined = doc(
            json!({"definition": {"entity": "SalesInvoice", "group_by": [{"field": "customer.territory"}], "measures": [{"fn": "sum", "field": "grand_total"}]}}),
        );
        assert!(dataset_spec(&joined).is_ok());
        let table = doc(json!({"definition": {"entity": "Customer", "fields": [{"field": "customer_name"}, {"field": "customer_group"}]}}));
        assert!(dataset_spec(&table).is_ok());
        let pivoted = doc(json!({"definition": {"entity": "Customer", "pivot": {"field": "customer_type"}, "measures": [{"fn": "count"}]}}));
        assert!(dataset_spec(&pivoted).is_err(), "columns from its rows");
        let named = doc(
            json!({"definition": {"entity": "Customer", "pivot": {"field": "customer_type", "values": ["Company", "Individual"]}, "measures": [{"fn": "count"}]}}),
        );
        assert!(dataset_spec(&named).is_ok());
        let compared = doc(json!({"definition": {"entity": "SalesInvoice", "measures": [{"fn": "count"}], "compare": "previous_year",
            "date_range": {"field": "posting_date", "from": "start_of_year", "to": "today"}}}));
        assert!(dataset_spec(&compared).is_ok());
        assert!(dataset_spec(&doc(json!({"definition": {"entity": "Customer"}}))).is_err(), "no columns");
        let cols = [
            json!({"name": "territory", "label": "Territory", "type": "data"}),
            json!({"name": "sum_grand_total", "label": "sum_grand_total", "type": "currency"}),
        ];
        assert_eq!(columns_text(&cols), "territory (Territory): data\nsum_grand_total: currency");
    }
}
