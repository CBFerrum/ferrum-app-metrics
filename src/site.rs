//! Metrics' endpoints, for the signed-in person (or an API key) with their own rights to the records
//! counted. `/api/app/metrics/value?metric=<name>`: a certified metric's value and what it means. A
//! metric in draft is read only by its owner; a deprecated one points at what replaces it.
//! `dataset?name=<name>&format=csv|ndjson` and `values?since=<time>` are the warehouse feed
//! (`warehouse`). `POST import/wren` reviews a Wren semantic model against the metrics defined here
//! and brings in what a reviewer accepts, as drafts (`wren`).

use ferrum_domain::util::sys;
use ferrum_kernel::{Error, Filter, Kernel, Result, Site, SiteRequest, SiteResponse};
use serde_json::{Value, json};

pub struct MetricsSite;

#[async_trait::async_trait]
impl Site for MetricsSite {
    async fn handle(&self, k: &Kernel, req: SiteRequest) -> Result<SiteResponse> {
        let user = req.user.as_ref().ok_or_else(|| Error::Unauthorized("sign in first".into()))?;
        let param = |key: &str| req.query.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
        let path = req.path.trim_matches('/').to_string();
        match (req.method.as_str(), path.as_str()) {
            ("GET", "value" | "series" | "impact") => {}
            ("GET", "dataset") => {
                let name = param("name").ok_or_else(|| Error::field("name", "is required"))?;
                return crate::warehouse::dataset(k, user, name, param("format").unwrap_or("csv")).await;
            }
            ("GET", "values") => return crate::warehouse::values(k, user, param("since")).await,
            ("POST", "import/wren") => {
                let mut r = SiteResponse::json(200, &crate::wren::import(k, user, &req.body).await?);
                r.headers.push(("cache-control".into(), "no-store".into()));
                return Ok(r);
            }
            _ => return Err(Error::NotFound(format!("metrics endpoint {} {}", req.method, req.path))),
        }
        let name =
            req.query.get("metric").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).ok_or_else(|| Error::field("metric", "is required"))?;
        let mut tx = k.db.tenant_tx(user.tenant).await?;
        let found = k.find_tx(&sys(user), &mut tx, "Metric", &[Filter::eq("metric_name", name)], None, 1).await?;
        tx.commit().await?;
        let metric = found.into_iter().next().ok_or_else(|| Error::NotFound("metric".into()))?;
        crate::metric::readable_state(k, user, "Metric", &metric).await?;
        let out = match path.as_str() {
            // Each period's value, the current one marked partial, and when it was worked out.
            "series" => {
                let bucket: ferrum_studio::spec::Bucket = serde_json::from_value(json!(param("bucket").unwrap_or("month")))
                    .map_err(|_| Error::field("bucket", "is day, week, month, quarter, year, fiscal_quarter or fiscal_year"))?;
                let periods: u32 = param("periods").map_or(Ok(12), str::parse).map_err(|_| Error::field("periods", "is a number"))?;
                crate::metric::series_json(k, user, &metric, bucket, periods).await?
            }
            // What a change to it reaches: the metrics composed of it, directly or by way of others.
            "impact" => crate::metric::impact_json(k, user, &metric).await?,
            // Its value, and the external datasets it was read from that are older than their sources allow.
            _ => crate::metric::value_json(k, user, &metric).await?,
        };
        let mut r = SiteResponse::json(200, &out);
        r.headers.push(("cache-control".into(), "no-store".into()));
        Ok(r)
    }
}
