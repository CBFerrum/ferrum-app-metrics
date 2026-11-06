//! Governed metrics (FERRUM-ECOSYSTEM-010) against a real PostgreSQL
//! (`FERRUM_METRICS_TEST_DATABASE_URL`, default `postgres://ferrum:ferrum@localhost:5432/ferrum_metrics_test`;
//! reset first): a metric defined once as a Studio query of one measure, refused when it does not
//! run; its lineage; certified only by a steward who does not own it; read by each person with their
//! own rights, so two sales reps and their director read three numbers from one definition; its
//! value kept as a snapshot; a changed definition back in draft until certified again; deprecation
//! pointing at what replaces it; the glossary; certified metrics delivered to who follows them,
//! each read with their own rights, once on their day, and a deprecated one said so once; and who
//! may do what.

use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings, ListQuery};
use ferrum_meta::Catalog;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn uid(v: &Value) -> Uuid {
    v.as_str().unwrap_or_else(|| panic!("not an id: {v}")).parse().unwrap()
}

fn dec(v: &Value) -> f64 {
    v.as_str().map(|s| s.parse().unwrap()).or_else(|| v.as_f64()).unwrap_or_else(|| panic!("not a number: {v}"))
}

struct T {
    k: Arc<Kernel>,
    tenant: Uuid,
    admin: Ctx,
}

impl T {
    async fn insert(&self, ctx: &Ctx, entity: &str, v: Value) -> Doc {
        self.k.insert(ctx, entity, obj(v)).await.unwrap_or_else(|e| panic!("insert {entity}: {e:?}"))
    }
    async fn get(&self, entity: &str, id: Uuid) -> Doc {
        self.k.get(&self.admin, entity, id, false).await.unwrap_or_else(|e| panic!("get {entity}: {e:?}"))
    }
    async fn user(&self, email: &str, roles: &[&str]) -> Ctx {
        let roles: Vec<String> = roles.iter().map(|r| r.to_string()).collect();
        let id = self.k.create_user(&self.admin, email, email, Some("Correct-Horse-9"), &roles, false).await.expect("user");
        let s = self.k.login("numbers", email, "Correct-Horse-9").await.expect("login");
        self.k.load_ctx(self.tenant, id, Some(s.token_version), "it".into()).await.expect("ctx")
    }
    async fn act(&self, ctx: &Ctx, id: Uuid, action: &str, params: Value) -> Result<Value, Error> {
        self.k.run_action(ctx, "Metric", id, action, params).await
    }
}

/// The value endpoint as `who` (`/api/app/metrics/value?metric=…`).
async fn read(k: &Kernel, who: &Ctx, metric: &str) -> Result<Value, Error> {
    let site = k.hooks.site("metrics").expect("metrics' endpoint").clone();
    let req = ferrum_kernel::SiteRequest {
        tenant: who.tenant,
        user: Some(who.clone()),
        method: "GET".into(),
        path: "/value".into(),
        query: json!({"metric": metric}).as_object().cloned().unwrap(),
        headers: Default::default(),
        body: vec![],
        client_ip: "203.0.113.9".into(),
    };
    let r = site.handle(k, req).await?;
    Ok(serde_json::from_slice(&r.body).unwrap())
}

fn refused<V: std::fmt::Debug>(r: Result<V, Error>, kind: &str, says: &str) {
    let msg = format!("{r:?}");
    let ok = matches!(
        (&r, kind),
        (Err(Error::Invalid(_) | Error::Validation(_)), "invalid") | (Err(Error::Forbidden(_)), "forbidden") | (Err(Error::NotFound(_)), "not found")
    );
    assert!(ok && msg.contains(says), "expected {kind} saying {says:?}, got {msg}");
}

#[test]
fn governed_metrics() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_METRICS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_metrics_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 16, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_metrics::packages().expect("app packages"));
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_metrics::register(&mut hooks);
    let settings = KernelSettings { secret_key: Some(SecretKey::new([7u8; 32])), ..Default::default() };
    let k = Kernel::boot(db, catalog, hooks, settings, None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "numbers".into(),
            name: "Numbers Inc".into(),
            admin_email: "admin@numbers.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["metrics".into(), "parties".into()],
            settings: Some(json!({"timezone": "UTC"})),
        })
        .await
        .expect("tenant");
    let session = k.login("numbers", "admin@numbers.test", "Correct-Horse-9").await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let t = T { k: k.clone(), tenant, admin };
    let a = t.admin.clone();
    // Sales reps see the customers they made; their director sees them all.
    let rule = |p: Value| serde_json::from_value::<ferrum_meta::PermissionDef>(p).unwrap();
    t.k.set_role_permission(&a, "Sales Rep", &rule(json!({"entity": "Customer", "read": true, "create": true, "if_owner": true}))).await.unwrap();
    t.k.set_role_permission(&a, "Sales Director", &rule(json!({"entity": "Customer", "read": true, "create": true}))).await.unwrap();
    let author = t.user("analyst@numbers.test", &["Metric Author", "Sales Director"]).await;
    let steward = t.user("steward@numbers.test", &["Data Steward", "Sales Director"]).await;
    let rep1 = t.user("rep1@numbers.test", &["Metric Reader", "Sales Rep"]).await;
    let rep2 = t.user("rep2@numbers.test", &["Metric Reader", "Sales Rep"]).await;
    let director = t.user("director@numbers.test", &["Metric Reader", "Sales Director"]).await;
    let customer = |name: &str, kind: &str, days: i64| json!({"customer_name": name, "customer_type": kind, "payment_terms_days": days});
    t.insert(&rep1, "Customer", customer("Acme", "Company", 30)).await;
    t.insert(&rep1, "Customer", customer("Globex", "Company", 60)).await;
    t.insert(&rep1, "Customer", customer("Ann", "Individual", 0)).await;
    t.insert(&rep2, "Customer", customer("Initech", "Company", 90)).await;

    // ───────────── a metric defined once ─────────────
    let term = uid(&t
        .insert(&author, "BusinessTerm", json!({"term": "Company customer", "definition": "A customer that is an organisation, not a person."}))
        .await["id"]);
    let def = |kind: &str| json!({"entity": "Customer", "filters": [["customer_type", "eq", kind]], "measures": [{"fn": "count", "label": "Customers"}]});
    let metric = |name: &str, definition: Value| json!({"metric_name": name, "domain": "Sales", "unit": "customers", "definition": definition, "terms": [{"term": term.to_string()}]});
    refused(
        t.k.insert(&author, "Metric", obj(metric("Two", json!({"entity": "Customer", "measures": [{"fn": "count"}, {"fn": "count"}]})))).await,
        "invalid",
        "one measure, ungrouped",
    );
    refused(
        t.k.insert(&author, "Metric", obj(metric("Nowhere", json!({"entity": "Unicorn", "measures": [{"fn": "count"}]})))).await,
        "invalid",
        "does not run",
    );
    refused(
        t.k.insert(&author, "Metric", obj(metric("Bad field", json!({"entity": "Customer", "measures": [{"fn": "sum", "field": "no_such_field"}]})))).await,
        "invalid",
        "does not run",
    );
    let m = t.insert(&author, "Metric", metric("Company customers", def("Company"))).await;
    assert_eq!((m["owner"].as_str(), m["uses"].as_str(), m["status"].as_str()), (Some("analyst@numbers.test"), Some("Customer: customer_type"), Some("Draft")));
    let mid = uid(&m["id"]);
    assert_eq!(dec(&read(&k, &author, "Company customers").await.unwrap()["value"]), 3.0, "its owner reads the draft");
    refused(read(&k, &rep1, "Company customers").await, "not found", "");

    // ───────────── certification ─────────────
    refused(t.act(&author, mid, "certify", Value::Null).await, "forbidden", "a Data Steward certifies");
    let owner_steward = t.user("both@numbers.test", &["Data Steward", "Metric Author", "Sales Director"]).await;
    let own = uid(&t.insert(&owner_steward, "Metric", metric("Own metric", def("Company"))).await["id"]);
    refused(t.act(&owner_steward, own, "certify", Value::Null).await, "forbidden", "someone other than its owner");
    t.act(&steward, mid, "certify", Value::Null).await.expect("certified");
    let c = t.get("Metric", mid).await;
    assert_eq!((c["status"].as_str(), c["certified_by"].as_str()), (Some("Certified"), Some("steward@numbers.test")));

    // ───────────── read with each person's rights ─────────────
    let v = |r: Value| dec(&r["value"]);
    assert_eq!(v(read(&k, &rep1, "Company customers").await.unwrap()), 2.0, "the two she made");
    assert_eq!(v(read(&k, &rep2, "Company customers").await.unwrap()), 1.0, "the one he made");
    let all = read(&k, &director, "Company customers").await.unwrap();
    assert_eq!((v(all.clone()), all["certified"].as_bool(), all["lineage"].as_str()), (3.0, Some(true), Some("Customer: customer_type")));
    let snap = t.act(&rep1, mid, "snapshot", Value::Null).await.expect("a value kept");
    assert_eq!(dec(&snap["value"]), 2.0);
    t.act(&director, mid, "snapshot", Value::Null).await.unwrap();
    let hers = t.k.list(&rep1, "MetricSnapshot", &ListQuery::default()).await.unwrap().rows;
    assert_eq!((hers.len(), hers[0]["read_by"].as_str()), (1, Some("rep1@numbers.test")), "a reader sees her own values");
    assert_eq!(t.k.list(&steward, "MetricSnapshot", &ListQuery::default()).await.unwrap().rows.len(), 2);
    // An average is a decimal, read the same way.
    let avg = json!({"entity": "Customer", "filters": [["customer_type", "eq", "Company"]], "measures": [{"fn": "avg", "field": "payment_terms_days"}]});
    let terms = uid(&t.insert(&author, "Metric", json!({"metric_name": "Average payment terms", "unit": "days", "definition": avg})).await["id"]);
    t.act(&steward, terms, "certify", Value::Null).await.unwrap();
    let days = |r: Value| (dec(&r["value"]), r["lineage"].as_str().map(str::to_string));
    assert_eq!(days(read(&k, &rep1, "Average payment terms").await.unwrap()), (45.0, Some("Customer: customer_type, payment_terms_days".into())));
    assert_eq!(days(read(&k, &rep2, "Average payment terms").await.unwrap()).0, 90.0);
    assert_eq!(days(read(&k, &director, "Average payment terms").await.unwrap()).0, 60.0);

    // ───────────── change control ─────────────
    t.k.update(&author, "Metric", mid, obj(json!({"description": "Organisations we sell to"})), None).await.unwrap();
    assert_eq!(t.get("Metric", mid).await["status"], "Certified", "its description is not its definition");
    t.k.update(&author, "Metric", mid, obj(json!({"definition": def("Individual")})), None).await.unwrap();
    let changed = t.get("Metric", mid).await;
    assert_eq!((changed["status"].as_str(), changed["certified_by"].as_str()), (Some("Draft"), None), "a new definition is a draft again");
    refused(read(&k, &rep1, "Company customers").await, "not found", "");
    let seen: Vec<Value> = t.k.list(&rep1, "Metric", &ListQuery::default()).await.unwrap().rows.iter().map(|m| m["metric_name"].clone()).collect();
    assert_eq!(seen, vec![json!("Average payment terms")], "readers see certified metrics only");
    t.act(&steward, mid, "certify", Value::Null).await.unwrap();
    assert_eq!(v(read(&k, &director, "Company customers").await.unwrap()), 1.0, "now it counts people");

    // ───────────── deprecation ─────────────
    let all_customers = uid(&t.insert(&author, "Metric", metric("All customers", json!({"entity": "Customer", "measures": [{"fn": "count"}]}))).await["id"]);
    t.act(&steward, all_customers, "certify", Value::Null).await.unwrap();
    refused(t.act(&author, mid, "deprecate", json!({"replaced_by": all_customers.to_string()})).await, "forbidden", "a Data Steward deprecates");
    refused(t.act(&steward, mid, "deprecate", json!({"replaced_by": mid.to_string()})).await, "invalid", "does not replace itself");
    t.act(&steward, mid, "deprecate", json!({"replaced_by": all_customers.to_string()})).await.unwrap();
    refused(read(&k, &author, "Company customers").await, "invalid", "deprecated: read All customers instead");
    assert_eq!(v(read(&k, &director, "All customers").await.unwrap()), 4.0);
    assert_eq!(v(read(&k, &rep2, "All customers").await.unwrap()), 1.0);

    // ───────────── delivered to who follows them ─────────────
    let follow = |metric: Uuid, frequency: &str| json!({"metric": metric.to_string(), "frequency": frequency});
    // The steward sees the deprecated one, and is refused it too.
    refused(t.k.insert(&steward, "MetricSubscription", obj(follow(mid, "Weekly"))).await, "invalid", "only a certified metric is followed");
    let hers = t.insert(&rep1, "MetricSubscription", follow(all_customers, "Weekly")).await;
    assert_eq!(hers["subscriber"], "rep1@numbers.test", "a person follows for themselves");
    refused(t.k.insert(&rep1, "MetricSubscription", obj(follow(all_customers, "Daily"))).await, "invalid", "you follow it already");
    let his = t.insert(&director, "MetricSubscription", follow(all_customers, "Daily")).await;
    let terms_follow = t.insert(&director, "MetricSubscription", follow(terms, "Monthly")).await;
    let told = |who: Uuid| {
        let k = k.clone();
        async move {
            let mut tx = k.db.tenant_tx(tenant).await.unwrap();
            let n: i64 = sqlx::query_scalar("select count(*) from notification where tenant_id = $1 and user_id = $2 and kind = 'metrics.digest'")
                .bind(tenant)
                .bind(who)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            n
        }
    };
    ferrum_app_metrics::maintenance(&k).await.expect("delivered");
    // Each read with their own rights, kept as a value, and told.
    let h = t.get("MetricSubscription", uid(&hers["id"])).await;
    let d = t.get("MetricSubscription", uid(&his["id"])).await;
    assert_eq!((dec(&h["last_value"]), dec(&d["last_value"])), (3.0, 4.0), "the three she made; all four");
    assert!(h["last_sent_on"].is_string() && h["last_error"].is_null(), "{h:?}");
    assert_eq!((told(rep1.user).await, told(director.user).await), (1, 2), "hers, and his two");
    let kept = t.k.list(&rep1, "MetricSnapshot", &ListQuery::default()).await.unwrap().rows;
    assert!(kept.iter().any(|s| dec(&s["value"]) == 3.0), "kept as a value of hers");
    // Not again until it is due.
    ferrum_app_metrics::maintenance(&k).await.expect("upkeep");
    assert_eq!((told(rep1.user).await, told(director.user).await), (1, 2), "once a day at most");
    // A metric deprecated since is said so once, and no longer delivered.
    t.act(&steward, terms, "deprecate", json!({"replaced_by": all_customers.to_string()})).await.unwrap();
    let mut tx = k.db.tenant_tx(tenant).await.unwrap();
    sqlx::query("update e_metric_subscription set last_sent_on = last_sent_on - 40 where tenant_id = $1 and id = $2")
        .bind(tenant)
        .bind(uid(&terms_follow["id"]))
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    ferrum_app_metrics::maintenance(&k).await.expect("upkeep");
    let gone = t.get("MetricSubscription", uid(&terms_follow["id"])).await;
    assert_eq!(gone["active"], false, "{gone:?}");
    assert_eq!(told(director.user).await, 3, "told it is deprecated, and what replaces it");
    // Someone else's is not theirs to see.
    assert_eq!(t.k.list(&rep2, "MetricSubscription", &ListQuery::default()).await.unwrap().rows.len(), 0);

    // ───────────── who may do what ─────────────
    refused(t.k.insert(&rep1, "Metric", obj(metric("Mine", def("Company")))).await, "forbidden", "");
    refused(t.k.insert(&rep1, "BusinessTerm", obj(json!({"term": "Mine", "definition": "x"}))).await, "forbidden", "");
}
