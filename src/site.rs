//! Metrics' endpoint (`/api/app/metrics/value?metric=<name>`): a certified metric's value for the
//! signed-in person, with their own rights to the records it counts, and what it means. A metric in
//! draft is read only by its owner; a deprecated one points at what replaces it.

use crate::metric::value;
use ferrum_domain::util::{id, s, sys};
use ferrum_kernel::{Error, Filter, Kernel, Result, Site, SiteRequest, SiteResponse};
use serde_json::{Value, json};

pub struct MetricsSite;

#[async_trait::async_trait]
impl Site for MetricsSite {
    async fn handle(&self, k: &Kernel, req: SiteRequest) -> Result<SiteResponse> {
        let user = req.user.as_ref().ok_or_else(|| Error::Unauthorized("sign in first".into()))?;
        if (req.method.as_str(), req.path.trim_matches('/')) != ("GET", "value") {
            return Err(Error::NotFound(format!("metrics endpoint {} {}", req.method, req.path)));
        }
        let name =
            req.query.get("metric").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).ok_or_else(|| Error::field("metric", "is required"))?;
        let mut tx = k.db.tenant_tx(user.tenant).await?;
        let found = k.find_tx(&sys(user), &mut tx, "Metric", &[Filter::eq("metric_name", name)], None, 1).await?;
        tx.commit().await?;
        let metric = found.into_iter().next().ok_or_else(|| Error::NotFound("metric".into()))?;
        match s(&metric, "status") {
            Some("Certified") => {}
            Some("Draft") if s(&metric, "owner").is_some_and(|o| o.eq_ignore_ascii_case(&user.email)) => {}
            Some("Deprecated") => {
                let instead = match id(&metric, "replaced_by") {
                    Some(r) => k.get(&sys(user), "Metric", r, false).await.ok().and_then(|m| s(&m, "metric_name").map(str::to_string)),
                    None => None,
                };
                return Err(Error::invalid(match instead {
                    Some(n) => format!("{name} is deprecated: read {n} instead"),
                    None => format!("{name} is deprecated"),
                }));
            }
            _ => return Err(Error::NotFound("metric".into())),
        }
        let v = value(k, user, &metric).await?;
        let out = json!({"metric": name, "value": ferrum_domain::util::money(v), "unit": s(&metric, "unit"), "certified": s(&metric, "status") == Some("Certified"),
                         "description": s(&metric, "description"), "lineage": s(&metric, "uses")});
        let mut r = SiteResponse::json(200, &out);
        r.headers.push(("cache-control".into(), "no-store".into()));
        Ok(r)
    }
}
