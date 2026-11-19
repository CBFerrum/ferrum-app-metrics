//! What Metrics' actions promise, as the gate serves it to people's screens, processes and agents alike
//! (`FERRUM_METRICS_CONTRACTS_TEST_DATABASE_URL`, emptied first): each says its effect, what it returns and what
//! "done" means.

use ferrum_testkit::{Kit, app, database, run};
use serde_json::json;

#[test]
fn contracts_are_served() {
    run(async {
        let kit = Kit::boot(&database("FERRUM_METRICS_CONTRACTS_TEST_DATABASE_URL"), &[app!("metrics", ferrum_app_metrics)]).await;
        let ws = kit.workspace("promises", &["metrics"], json!({"timezone": "UTC"})).await;
        for (id, says) in [
            ("metrics.metric.certify", "Certified"),
            ("metrics.metric.deprecate", "Deprecated"),
            ("metrics.metric.snapshot", "Metric Value"),
            ("metrics.dataset.certify", "Certified"),
            ("metrics.dataset.deprecate", "Deprecated"),
        ] {
            let cap = kit.k.capability(&ws.admin, id).unwrap_or_else(|e| panic!("{id}: {e:?}"));
            assert_eq!(format!("{:?}", cap.effect), "TransactionalAction", "{id}");
            assert!(cap.completion.contains(says), "{id}: {}", cap.completion);
            assert!(cap.output["properties"].as_object().is_some_and(|p| !p.is_empty()), "{id} says what it returns: {}", cap.output);
        }
        let deprecate = kit.k.capability(&ws.admin, "metrics.metric.deprecate").unwrap();
        assert!(deprecate.input.to_string().contains("replaced_by"), "its parameter is declared: {}", deprecate.input);
    });
}
