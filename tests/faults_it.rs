//! Faults found on 19 November 2026, each proved fixed against a real PostgreSQL
//! (`FERRUM_METRICS_FAULTS_TEST_DATABASE_URL`, emptied first): who may read a metric or a dataset through Metrics'
//! endpoints, what impact names, a digest delivered once however many runners read it due, one active follow per
//! person and metric, and a round that stops at its budget.

use ferrum_kernel::{Ctx, Error, Kernel};
use ferrum_testkit::{Kit, Workspace, app, database, dec, obj, refused, run, uid};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;
use uuid::Uuid;

static KIT: OnceCell<Kit> = OnceCell::const_new();

async fn kit() -> &'static Kit {
    KIT.get_or_init(|| async { Kit::boot(&database("FERRUM_METRICS_FAULTS_TEST_DATABASE_URL"), &[app!("metrics", ferrum_app_metrics)]).await }).await
}

/// A workspace with sales reps' customers, certified metrics and the people who read them.
struct Shop {
    ws: Workspace,
    author: Ctx,
    steward: Ctx,
    reader: Ctx,
    /// Signed in, with no Metrics role.
    outsider: Ctx,
    /// Reads only the Sales domain's metrics.
    narrow: Ctx,
    customers: Uuid,
}

impl Shop {
    fn k(&self) -> &Kernel {
        &self.ws.k
    }

    async fn certify(&self, entity: &str, id: Uuid) {
        self.k().run_action(&self.steward, entity, id, "certify", Value::Null).await.unwrap_or_else(|e| panic!("certify {entity}: {e:?}"));
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
        let r = site.handle(self.k(), req).await?;
        Ok(serde_json::from_slice(&r.body).unwrap_or_else(|_| json!({"text": String::from_utf8_lossy(&r.body)})))
    }

    /// How many notifications of `kind` `who` has.
    async fn told(&self, who: &Ctx, kind: &str) -> i64 {
        let mut tx = self.k().db.tenant_tx(self.ws.tenant).await.unwrap();
        let n = sqlx::query_scalar("select count(*) from notification where tenant_id = $1 and user_id = $2 and kind = $3")
            .bind(self.ws.tenant)
            .bind(who.user)
            .bind(kind)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        n
    }

    async fn count(&self, sql: &'static str) -> i64 {
        let mut tx = self.k().db.tenant_tx(self.ws.tenant).await.unwrap();
        let n = sqlx::query_scalar(sql).bind(self.ws.tenant).fetch_one(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        n
    }
}

async fn shop(slug: &str) -> Shop {
    let ws = kit().await.workspace(slug, &["metrics", "parties"], json!({"timezone": "UTC"})).await;
    let k = ws.k.clone();
    let rule = |p: Value| serde_json::from_value::<ferrum_meta::PermissionDef>(p).unwrap();
    k.set_role_permission(&ws.admin, "Sales Rep", &rule(json!({"entity": "Customer", "read": true, "create": true, "if_owner": true}))).await.unwrap();
    k.set_role_permission(&ws.admin, "Sales Metrics", &rule(json!({"entity": "Metric", "read": true, "where": {"domain": "Sales"}}))).await.unwrap();
    let person = |who: &str| format!("{who}@{slug}.test");
    let author = ws.user(&person("author"), &["Metric Author", "Sales Rep"]).await;
    let steward = ws.user(&person("steward"), &["Data Steward", "Sales Rep"]).await;
    let reader = ws.user(&person("reader"), &["Metric Reader", "Sales Rep"]).await;
    let outsider = ws.user(&person("outsider"), &["Sales Rep"]).await;
    let narrow = ws.user(&person("narrow"), &["Sales Metrics", "Sales Rep"]).await;
    for name in ["Acme", "Globex"] {
        k.insert(&reader, "Customer", obj(json!({"customer_name": name, "customer_type": "Company"}))).await.unwrap();
    }
    let count = json!({"entity": "Customer", "measures": [{"fn": "count"}]});
    let metric = |name: &str, domain: &str| obj(json!({"metric_name": name, "domain": domain, "definition": count}));
    let customers = uid(&k.insert(&author, "Metric", metric("Customers", "Sales")).await.unwrap());
    let finance = uid(&k.insert(&author, "Metric", metric("Finance customers", "Finance")).await.unwrap());
    let s = Shop { ws, author, steward, reader, outsider, narrow, customers };
    s.certify("Metric", customers).await;
    s.certify("Metric", finance).await;
    s
}

#[test]
fn value_refused_without_a_metrics_role() {
    run(async {
        let s = shop("norole").await;
        for path in ["/value", "/series", "/impact"] {
            refused(s.site(&s.outsider, path, json!({"metric": "Customers"})).await, "forbidden", "needs a Metrics role");
        }
        assert_eq!(dec(&s.site(&s.reader, "/value", json!({"metric": "Customers"})).await.unwrap()["value"]), 2.into(), "a reader still reads it");
    });
}

#[test]
fn value_follows_a_narrower_row_rule() {
    run(async {
        let s = shop("narrow").await;
        assert!(s.site(&s.narrow, "/value", json!({"metric": "Customers"})).await.is_ok(), "a Sales metric is theirs to read");
        refused(s.site(&s.narrow, "/value", json!({"metric": "Finance customers"})).await, "not found", "");
    });
}

#[test]
fn dataset_feed_refused_without_a_metrics_role() {
    run(async {
        let s = shop("feed").await;
        let set = json!({"dataset_name": "Names", "definition": {"entity": "Customer", "fields": [{"field": "customer_name"}]}});
        let id = uid(&s.k().insert(&s.author, "Dataset", obj(set)).await.unwrap());
        s.certify("Dataset", id).await;
        refused(s.site(&s.outsider, "/dataset", json!({"name": "Names"})).await, "forbidden", "needs a Metrics role");
        assert!(s.site(&s.reader, "/dataset", json!({"name": "Names"})).await.is_ok(), "a reader still loads it");
    });
}

#[test]
fn impact_names_only_metrics_the_caller_reads() {
    run(async {
        let s = shop("impact").await;
        s.k().insert(&s.author, "Metric", obj(json!({"metric_name": "Customers twice", "formula": "[Customers] * 2"}))).await.unwrap();
        let used = |v: Value| v["used_by"].clone();
        assert_eq!(used(s.site(&s.author, "/impact", json!({"metric": "Customers"})).await.unwrap()), json!(["Customers twice"]), "its author reads the draft");
        assert_eq!(used(s.site(&s.reader, "/impact", json!({"metric": "Customers"})).await.unwrap()), json!([]), "a reader is not told of a draft");
    });
}

#[test]
fn digest_delivered_once_across_two_runners() {
    run(async {
        let s = shop("twice").await;
        s.k().insert(&s.reader, "MetricSubscription", obj(json!({"metric": s.customers.to_string(), "frequency": "Daily"}))).await.unwrap();
        // Two runners read the same subscriptions due before either delivers.
        let due = ferrum_app_metrics::digests_due(s.k(), s.ws.tenant).await.unwrap();
        assert_eq!(due.len(), 1);
        let later = Instant::now() + Duration::from_secs(60);
        ferrum_app_metrics::deliver_due(s.k(), s.ws.tenant, due.clone(), later).await.unwrap();
        ferrum_app_metrics::deliver_due(s.k(), s.ws.tenant, due, later).await.unwrap();
        assert_eq!(s.told(&s.reader, "metrics.digest").await, 1, "told once");
        assert_eq!(s.count("select count(*) from e_metric_snapshot where tenant_id = $1").await, 1, "kept once");
    });
}

#[test]
fn a_digest_round_stops_at_its_budget() {
    run(async {
        let s = shop("budget").await;
        s.k().insert(&s.reader, "MetricSubscription", obj(json!({"metric": s.customers.to_string(), "frequency": "Daily"}))).await.unwrap();
        let due = ferrum_app_metrics::digests_due(s.k(), s.ws.tenant).await.unwrap();
        assert_eq!(ferrum_app_metrics::deliver_due(s.k(), s.ws.tenant, due, Instant::now()).await.unwrap(), 0, "nothing started past the budget");
        assert_eq!(s.told(&s.reader, "metrics.digest").await, 0);
        // The next round delivers it.
        ferrum_app_metrics::maintenance(s.k(), s.ws.tenant).await.unwrap();
        assert_eq!(s.told(&s.reader, "metrics.digest").await, 1);
    });
}

/// A second active follow, written past the hook, is refused by the database; and duplicates a racy check let in
/// before the index existed are stopped, leaving the first, so the index builds.
#[test]
fn follows_are_one_active_per_person_and_metric() {
    run(async {
        let s = shop("once").await;
        let first = uid(&s.k().insert(&s.reader, "MetricSubscription", obj(json!({"metric": s.customers.to_string()}))).await.unwrap());
        // The first follow copied under a new id, past the hook.
        let twice = async || -> Result<(), sqlx::Error> {
            let mut tx = s.k().db.tenant_tx(s.ws.tenant).await.unwrap();
            sqlx::query("create temp table f on commit drop as select * from e_metric_subscription where tenant_id = $1 and id = $2")
                .bind(s.ws.tenant)
                .bind(first)
                .execute(&mut *tx)
                .await?;
            sqlx::query("update f set id = gen_random_uuid()").execute(&mut *tx).await?;
            sqlx::query("insert into e_metric_subscription select * from f").execute(&mut *tx).await?;
            tx.commit().await.map_err(|e| sqlx::Error::Protocol(e.to_string()))
        };
        let e = twice().await.expect_err("a second active follow");
        assert!(e.to_string().contains("metrics_follow_once"), "{e}");
        // As a workspace may hold them from before: the index gone, a duplicate in.
        let mut tx = s.k().db.tenant_tx(s.ws.tenant).await.unwrap();
        sqlx::query("drop index metrics_follow_once").execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        twice().await.expect("a duplicate, with no index");
        let mut tx = s.k().db.tenant_tx(s.ws.tenant).await.unwrap();
        sqlx::raw_sql(include_str!("../packages/metrics/sql/001_metrics.sql")).execute(&mut *tx).await.expect("the file applies over duplicates");
        tx.commit().await.unwrap();
        let active = s.count("select count(*) from e_metric_subscription where tenant_id = $1 and active").await;
        assert_eq!(active, 1, "the later one stopped");
        assert_eq!(s.k().get(&s.ws.admin, "MetricSubscription", first, false).await.unwrap()["active"], true, "the first kept");
        assert!(twice().await.is_err(), "and the index is back");
    });
}
