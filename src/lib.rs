//! Governed analytics (FERRUM-ECOSYSTEM-010) on Studio: metrics defined once, certified by a
//! steward, kept under change control with their lineage, read with each person's own rights, and
//! a business glossary.
//!
//! * `metric`: a definition of one measure checked against the records and its author's rights;
//!   certification by a steward who does not own it, undone by a change of definition; values
//!   read with the reader's rights; snapshots kept; deprecation pointing at what replaces it.
//! * `site`: `/api/app/metrics/value`, a metric's value for whoever asks.
//!
//! This app's own design; no code was copied (docs/PROVENANCE.md). Hooks are registered per entity,
//! so a workspace without the `metrics` package never runs them.

mod metric;
mod site;

use ferrum_domain::reports::ReportSource;
use ferrum_kernel::{Doc, HookRegistry, Kernel, Result};
use serde_json::Value;
use std::sync::Arc;

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
    hooks.register("Metric", Arc::new(metric::MetricHook));
    hooks.add_site("metrics", Arc::new(site::MetricsSite));
}

/// This app's reports: none of its own yet.
pub fn reports() -> Vec<Arc<dyn ReportSource>> {
    vec![]
}

/// Upkeep: nothing runs by itself.
pub async fn maintenance(_k: &Kernel) -> Result<()> {
    Ok(())
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
