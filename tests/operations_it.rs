//! Metrics' reads as operations of the gate (`metrics.metric.value`, `.series`, `.impact`), against a real PostgreSQL
//! (`FERRUM_METRICS_OPERATIONS_TEST_DATABASE_URL`, emptied first): what people, the builder's tiles, processes and
//! agents over MCP all call, with the caller's own rights, writing nothing.

use ferrum_kernel::capability::Invocation;
use ferrum_kernel::{Ctx, Error, Kernel};
use ferrum_testkit::{Kit, Workspace, app, database, dec, obj, refused, run, uid};
use serde_json::{Value, json};
use tokio::sync::OnceCell;
use uuid::Uuid;

static KIT: OnceCell<Kit> = OnceCell::const_new();

async fn kit() -> &'static Kit {
    KIT.get_or_init(|| async { Kit::boot(&database("FERRUM_METRICS_OPERATIONS_TEST_DATABASE_URL"), &[app!("metrics", ferrum_app_metrics)]).await }).await
}

struct Shop {
    ws: Workspace,
    author: Ctx,
    steward: Ctx,
    rep1: Ctx,
    rep2: Ctx,
    customers: Uuid,
}

impl Shop {
    fn k(&self) -> &Kernel {
        &self.ws.k
    }

    /// `metrics.metric.<op>` on `id` as `who`, through the gate.
    async fn op(&self, who: &Ctx, op: &str, id: Uuid, params: Value) -> Result<Value, Error> {
        // The gate takes an action's parameters as an object, an empty one when it asks for none.
        let params = if params.is_null() { json!({}) } else { params };
        let input = json!({"id": id.to_string(), "params": params});
        self.k().invoke(who, Invocation::new(format!("metrics.metric.{op}"), input)).await.map(|r| r.result)
    }

    /// One of Metrics' endpoints as `who`.
    async fn site(&self, who: &Ctx, path: &str, query: Value) -> Result<Value, Error> {
        let site = self.k().hooks.site("metrics").expect("metrics' endpoint").clone();
        let req = ferrum_kernel::SiteRequest {
            tenant: who.tenant,
            user: Some(who.clone()),
            method: "GET".into(),
            path: path.into(),
            query: query.as_object().cloned().unwrap_or_default(),
            headers: Default::default(),
            body: vec![],
            client_ip: "203.0.113.9".into(),
        };
        Ok(serde_json::from_slice(&site.handle(self.k(), req).await?.body).unwrap())
    }
}

async fn shop(slug: &str) -> Shop {
    let ws = kit().await.workspace(slug, &["metrics", "parties"], json!({"timezone": "UTC"})).await;
    let k = ws.k.clone();
    let rule = |p: Value| serde_json::from_value::<ferrum_meta::PermissionDef>(p).unwrap();
    k.set_role_permission(&ws.admin, "Sales Rep", &rule(json!({"entity": "Customer", "read": true, "create": true, "if_owner": true}))).await.unwrap();
    let person = |who: &str| format!("{who}@{slug}.test");
    let author = ws.user(&person("author"), &["Metric Author", "Sales Rep"]).await;
    let steward = ws.user(&person("steward"), &["Data Steward", "Sales Rep"]).await;
    let rep1 = ws.user(&person("rep1"), &["Metric Reader", "Sales Rep"]).await;
    let rep2 = ws.user(&person("rep2"), &["Metric Reader", "Sales Rep"]).await;
    for (who, name) in [(&rep1, "Acme"), (&rep1, "Globex"), (&rep2, "Initech")] {
        k.insert(who, "Customer", obj(json!({"customer_name": name}))).await.unwrap();
    }
    let def = json!({"entity": "Customer", "measures": [{"fn": "count"}]});
    let customers = uid(&k
        .insert(&author, "Metric", obj(json!({"metric_name": "Customers", "unit": "customers", "definition": def, "series_by": "created_at"})))
        .await
        .unwrap());
    k.run_action(&steward, "Metric", customers, "certify", Value::Null).await.unwrap();
    Shop { ws, author, steward, rep1, rep2, customers }
}

#[test]
fn value_is_a_query_capability() {
    run(async {
        let s = shop("query").await;
        for op in ["value", "series", "impact"] {
            let cap = s.k().capability(&s.rep1, &format!("metrics.metric.{op}")).unwrap_or_else(|e| panic!("{op}: {e:?}"));
            assert_eq!(format!("{:?}", cap.effect), "Query", "{op}");
        }
        // Asked without an effect key, as a read is.
        assert!(s.op(&s.rep1, "value", s.customers, Value::Null).await.is_ok());
    });
}

#[test]
fn value_reads_with_the_callers_rights() {
    run(async {
        let s = shop("rights").await;
        let v = |r: Value| dec(&r["value"]);
        assert_eq!(v(s.op(&s.rep1, "value", s.customers, Value::Null).await.unwrap()), 2.into(), "the two she made");
        assert_eq!(v(s.op(&s.rep2, "value", s.customers, Value::Null).await.unwrap()), 1.into(), "the one he made");
        let r = s.op(&s.rep1, "value", s.customers, Value::Null).await.unwrap();
        assert_eq!((r["certified"].as_bool(), r["unit"].as_str(), r["lineage"].as_str()), (Some(true), Some("customers"), Some("Customer")));
        assert_eq!(s.ws.k.list(&s.ws.admin, "MetricSnapshot", &Default::default()).await.unwrap().rows.len(), 0, "nothing kept: a read writes nothing");
    });
}

#[test]
fn draft_value_only_for_its_owner() {
    run(async {
        let s = shop("draft").await;
        let def = json!({"entity": "Customer", "measures": [{"fn": "count"}]});
        let draft = uid(&s.k().insert(&s.author, "Metric", obj(json!({"metric_name": "Drafted", "definition": def}))).await.unwrap());
        assert!(s.op(&s.author, "value", draft, Value::Null).await.is_ok(), "its owner tries it");
        let other = s.ws.user("author2@draft.test", &["Metric Author", "Sales Rep"]).await;
        refused(s.op(&other, "value", draft, Value::Null).await, "not found", "");
        refused(s.op(&s.rep1, "value", draft, Value::Null).await, "not found", "");
    });
}

#[test]
fn deprecated_value_names_its_replacement() {
    run(async {
        let s = shop("old").await;
        let def = json!({"entity": "Customer", "measures": [{"fn": "count"}]});
        let all = uid(&s.k().insert(&s.author, "Metric", obj(json!({"metric_name": "All customers", "definition": def}))).await.unwrap());
        s.k().run_action(&s.steward, "Metric", all, "certify", Value::Null).await.unwrap();
        s.k().run_action(&s.steward, "Metric", s.customers, "deprecate", json!({"replaced_by": all.to_string()})).await.unwrap();
        refused(s.op(&s.author, "value", s.customers, Value::Null).await, "invalid", "deprecated: read All customers instead");
    });
}

#[test]
fn series_matches_the_endpoint() {
    run(async {
        let s = shop("series").await;
        let through = s.op(&s.rep1, "series", s.customers, json!({"bucket": "month", "periods": 3})).await.unwrap();
        let site = s.site(&s.rep1, "/series", json!({"metric": "Customers", "bucket": "month", "periods": "3"})).await.unwrap();
        assert_eq!(through["points"], site["points"]);
        assert_eq!(through["points"].as_array().map(Vec::len), Some(3));
        assert_eq!(through["points"][2]["partial"], true, "the current month is so far");
    });
}

#[test]
fn impact_through_the_gate_names_only_readable_metrics() {
    run(async {
        let s = shop("reach").await;
        s.k().insert(&s.author, "Metric", obj(json!({"metric_name": "Customers twice", "formula": "[Customers] * 2"}))).await.unwrap();
        assert_eq!(s.op(&s.author, "impact", s.customers, Value::Null).await.unwrap()["used_by"], json!(["Customers twice"]));
        assert_eq!(s.op(&s.rep1, "impact", s.customers, Value::Null).await.unwrap()["used_by"], json!([]));
    });
}
