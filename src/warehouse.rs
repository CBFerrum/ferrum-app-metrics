//! The warehouse feed. A data warehouse, or any loader that reads CSV or JSON lines over HTTPS,
//! pulls what Metrics governs, signed in with an API key of the person whose rights it loads with:
//!
//! * `GET /api/app/metrics/dataset?name=<name>&format=csv|ndjson`: a certified dataset's rows, its
//!   columns as certified; the definition's fingerprint in `x-ferrum-dataset-hash`, so a load can
//!   tell the definition changed, and `x-ferrum-truncated` when the rows were cut short.
//! * `GET /api/app/metrics/values?since=<time>`: certified metrics' kept values after `since`,
//!   oldest first, as JSON lines; `x-ferrum-next-since` is where the next load starts.
//!
//! A draft dataset is read by its owner only, to try it; a deprecated one says what replaces it.

use crate::dataset::rows;
use ferrum_domain::util::{id, s, sys};
use ferrum_kernel::{Ctx, Error, Filter, Kernel, ListQuery, Result, SiteResponse};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use uuid::Uuid;

/// The most values one load of the values feed gives.
const MOST_VALUES: u32 = 10_000;

/// A cell as CSV: quoted when it must be, and a text that a spreadsheet would run as a formula
/// made plain text. Numbers, and decimals kept as text, stay as they are.
fn cell(v: &Value) -> String {
    let text = match v {
        Value::Null => return String::new(),
        Value::String(t) => t.clone(),
        other => other.to_string(),
    };
    let formula = text.starts_with(['=', '+', '-', '@', '\t', '\r']) && text.parse::<f64>().is_err();
    let text = if formula { format!("'{text}") } else { text };
    if text.contains([',', '"', '\n', '\r']) { format!("\"{}\"", text.replace('"', "\"\"")) } else { text }
}

/// Rows as CSV, a header of the columns' names first.
pub(crate) fn csv(columns: &[Value], rows: &[Value]) -> String {
    let names: Vec<&str> = columns.iter().filter_map(|c| c["name"].as_str()).collect();
    let mut out = names.iter().map(|n| cell(&json!(n))).collect::<Vec<_>>().join(",");
    out.push_str("\r\n");
    for r in rows {
        out.push_str(&names.iter().map(|n| cell(&r[*n])).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}

/// Rows as JSON lines, each the columns' values by name, exactly as read.
pub(crate) fn ndjson(columns: &[Value], rows: &[Value]) -> String {
    let names: Vec<&str> = columns.iter().filter_map(|c| c["name"].as_str()).collect();
    rows.iter()
        .map(|r| {
            let line: Map<String, Value> = names.iter().map(|n| (n.to_string(), r[*n].clone())).collect();
            format!("{}\n", Value::Object(line))
        })
        .collect()
}

/// `dataset?name=`: a dataset's rows for `user`, as `format` asks.
pub(crate) async fn dataset(k: &Kernel, user: &Ctx, name: &str, format: &str) -> Result<SiteResponse> {
    let mut tx = k.db.tenant_tx(user.tenant).await?;
    let found = k.find_tx(&sys(user), &mut tx, "Dataset", &[Filter::eq("dataset_name", name)], None, 1).await?;
    tx.commit().await?;
    let set = found.into_iter().next().ok_or_else(|| Error::NotFound("dataset".into()))?;
    match s(&set, "status") {
        Some("Certified") => {}
        Some("Draft") if s(&set, "owner").is_some_and(|o| o.eq_ignore_ascii_case(&user.email)) => {}
        Some("Deprecated") => {
            let instead = match id(&set, "replaced_by") {
                Some(r) => k.get(&sys(user), "Dataset", r, false).await.ok().and_then(|d| s(&d, "dataset_name").map(str::to_string)),
                None => None,
            };
            return Err(Error::invalid(match instead {
                Some(n) => format!("{name} is deprecated: load {n} instead"),
                None => format!("{name} is deprecated"),
            }));
        }
        _ => return Err(Error::NotFound("dataset".into())),
    }
    let (columns, list, truncated) = rows(k, user, &set).await?;
    let (ctype, body, ext) = match format {
        "csv" => ("text/csv; charset=utf-8", csv(&columns, &list), "csv"),
        "ndjson" => ("application/x-ndjson", ndjson(&columns, &list), "ndjson"),
        _ => return Err(Error::field("format", "is csv or ndjson")),
    };
    let file: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let mut r = SiteResponse::text(200, ctype, body);
    r.headers.push(("cache-control".into(), "no-store".into()));
    r.headers.push(("content-disposition".into(), format!("attachment; filename=\"{file}.{ext}\"")));
    r.headers.push(("x-ferrum-dataset-hash".into(), s(&set, "certified_hash").unwrap_or("draft").to_string()));
    r.headers.push(("x-ferrum-truncated".into(), truncated.to_string()));
    Ok(r)
}

/// `values?since=`: certified metrics' kept values `user` may read, after `since`, oldest first.
pub(crate) async fn values(k: &Kernel, user: &Ctx, since: Option<&str>) -> Result<SiteResponse> {
    let since = match since.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => Some(chrono::DateTime::parse_from_rfc3339(v).map_err(|_| Error::field("since", "is a time like 2026-11-06T09:00:00Z"))?),
        None => None,
    };
    let filters = match since {
        Some(t) => ferrum_kernel::query::parse_filters(&json!([["read_at", ">", t.to_rfc3339()]]))?,
        None => vec![],
    };
    let q = ListQuery { filters, sort: Some("read_at asc".into()), limit: Some(MOST_VALUES), ..Default::default() };
    let kept = k.list(user, "MetricSnapshot", &q).await?.rows;
    // The metrics' names and units, whoever may read the values.
    let ids: Vec<Uuid> = kept.iter().filter_map(|v| id(v, "metric")).collect();
    let mut named: HashMap<Uuid, (Option<String>, Option<String>)> = HashMap::new();
    for m in ids {
        if let std::collections::hash_map::Entry::Vacant(e) = named.entry(m) {
            let doc = k.get(&sys(user), "Metric", m, false).await?;
            e.insert((s(&doc, "metric_name").map(str::to_string), s(&doc, "unit").map(str::to_string)));
        }
    }
    let mut next = since.map(|t| t.to_rfc3339());
    let mut body = String::new();
    for v in &kept {
        let (metric, unit) = id(v, "metric").and_then(|m| named.get(&m).cloned()).unwrap_or_default();
        let line = json!({"metric": metric, "value": v.get("value"), "unit": unit, "read_at": v.get("read_at"), "definition_hash": v.get("certified_hash")});
        body.push_str(&format!("{line}\n"));
        next = s(v, "read_at").map(str::to_string).or(next);
    }
    let mut r = SiteResponse::text(200, "application/x-ndjson", body);
    r.headers.push(("cache-control".into(), "no-store".into()));
    if let Some(n) = next {
        r.headers.push(("x-ferrum-next-since".into(), n));
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_load_as_csv_and_json_lines() {
        let columns = [json!({"name": "territory"}), json!({"name": "total"}), json!({"name": "note"})];
        let rows = [
            json!({"territory": "North, East", "total": "-120.50", "note": "=HYPERLINK(\"x\")"}),
            json!({"territory": "South", "total": 3, "note": null, "_titles": {}}),
        ];
        assert_eq!(csv(&columns, &rows), "territory,total,note\r\n\"North, East\",-120.50,\"'=HYPERLINK(\"\"x\"\")\"\r\nSouth,3,\r\n");
        let lines: Vec<Value> = ndjson(&columns, &rows).lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(
            lines,
            vec![json!({"territory": "North, East", "total": "-120.50", "note": "=HYPERLINK(\"x\")"}), json!({"territory": "South", "total": 3, "note": null})],
            "exactly as read, without titles"
        );
    }
}
