//! Governed analytics (FERRUM-ECOSYSTEM-010) on Studio: metrics and datasets defined once, certified
//! by a steward, kept under change control with their lineage, read with each person's own rights,
//! loaded into a warehouse, and a business glossary.
//!
//! * `metric`: a definition of one measure checked against the records and its author's rights;
//!   certification by a steward who does not own it, undone by a change of definition; values
//!   read with the reader's rights; snapshots kept; deprecation pointing at what replaces it.
//! * `dataset`: a table defined as a Studio query across linked records, governed as a metric is,
//!   its columns kept when it is certified.
//! * `warehouse`: a dataset's rows as CSV or JSON lines, and metrics' kept values since a time, for
//!   a warehouse to load with an API key's rights.
//! * `site`: `/api/app/metrics/value`, a metric's value for whoever asks, and the warehouse feed.
//! * `delivery`: certified metrics delivered to whoever follows them, daily, weekly or monthly,
//!   read with their own rights and kept as values.
//! * `wren`: a Wren semantic model reviewed against the metrics defined here, its accepted measures
//!   brought in as drafts; never a second authority, never an approximation.
//!
//! This app's own design; no code was copied (docs/PROVENANCE.md). Hooks are registered per entity,
//! so a workspace without the `metrics` package never runs them.

mod dataset;
mod delivery;
mod metric;
mod site;
mod warehouse;
mod wren;

use ferrum_domain::reports::ReportSource;
use ferrum_kernel::{Doc, Error, HookRegistry, Kernel, Result};
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

include!(concat!(env!("OUT_DIR"), "/packages.rs"));
include!(concat!(env!("OUT_DIR"), "/ui.rs"));

/// This app's manifest (`app.yaml`).
pub const MANIFEST: &str = include_str!("../app.yaml");

/// This app's metadata packages, embedded at build time.
pub fn packages() -> std::result::Result<Vec<ferrum_meta::Package>, ferrum_meta::MetaError> {
    ferrum_meta::catalog::load_embedded(EMBEDDED)
}

/// Registers this app's document hooks, and its endpoint.
pub fn register(hooks: &mut HookRegistry) {
    hooks
        .register("Metric", Arc::new(metric::MetricHook))
        .register("Dataset", Arc::new(dataset::DatasetHook))
        .register("MetricSubscription", Arc::new(delivery::SubscriptionHook));
    hooks.add_site("metrics", Arc::new(site::MetricsSite));
}

/// This app's reports: none of its own yet.
pub fn reports() -> Vec<Arc<dyn ReportSource>> {
    vec![]
}

/// Upkeep: each workspace's followed metrics delivered on the day they are due.
pub async fn maintenance(k: &Kernel) -> Result<()> {
    let mut tx = k.db.system_tx().await?;
    let tenants: Vec<Uuid> = sqlx::query_scalar("select id from tenant where status = 'active' order by id").fetch_all(&mut *tx).await?;
    tx.commit().await?;
    let mut first_error: Option<Error> = None;
    for tenant in tenants {
        if let Err(e) = delivery::deliver(k, tenant).await {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// A document from JSON.
pub(crate) fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interface_is_a_release_the_platform_takes() {
        let m = ferrum_apps::AppManifest::parse(MANIFEST).expect("app.yaml");
        assert_eq!(m.routes(), vec![("kpis".to_string(), vec![])]);
        let files = UI.iter().map(|(p, b)| (p.to_string(), b.to_vec())).collect();
        let ui = ferrum_apps::ui::prepare("metrics", files).expect("ui/dist is an interface release");
        assert!(UI.iter().any(|(p, _)| *p == "index.html") && !UI.iter().any(|(p, _)| p.starts_with('.')), "{}", ui.digest);
    }
}
