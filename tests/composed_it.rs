//! Composed metrics and series (Ferrum's Logic Platform, W6): a metric made of others by a formula,
//! refused when it would be made of itself, certified only after its parts, read by each person from
//! their own values of the parts; what a change to a metric reaches; and a metric period by period with
//! the period not yet over marked partial. Against a real PostgreSQL (`FERRUM_METRICS_TEST_DATABASE_URL`).

use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings};
use ferrum_meta::Catalog;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn uid(v: &Value) -> Uuid {
    v.as_str().unwrap().parse().unwrap()
}

async fn ask(k: &Kernel, who: &Ctx, path: &str, query: Value) -> Result<Value, Error> {
    let site = k.hooks.site("metrics").expect("metrics' endpoint").clone();
    let req = ferrum_kernel::SiteRequest {
        tenant: who.tenant,
        user: Some(who.clone()),
        method: "GET".into(),
        path: path.into(),
        query: query.as_object().cloned().unwrap(),
        headers: Default::default(),
        body: vec![],
        client_ip: "203.0.113.9".into(),
    };
    let r = site.handle(k, req).await?;
    Ok(serde_json::from_slice(&r.body).unwrap())
}

async fn value(k: &Kernel, who: &Ctx, metric: &str) -> Result<f64, Error> {
    let v = ask(k, who, "/value", json!({"metric": metric})).await?;
    assert!(v["as_of"].is_string(), "a value says when it was worked out: {v}");
    Ok(v["value"].as_str().map(|s| s.parse().unwrap()).or_else(|| v["value"].as_f64()).unwrap())
}

fn refused<V: std::fmt::Debug>(r: Result<V, Error>, says: &str) {
    let msg = format!("{r:?}");
    assert!(r.is_err() && msg.contains(says), "expected a refusal saying {says:?}, got {msg}");
}

#[test]
fn composed_metrics_and_series() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_METRICS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_metrics_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 16, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_metrics::packages().expect("app packages"));
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_metrics::register(&mut hooks);
    let settings = KernelSettings { secret_key: Some(SecretKey::new([7u8; 32])), ..Default::default() };
    let k: Arc<Kernel> = Kernel::boot(db, Catalog::from_packages(packages).expect("catalog"), hooks, settings, None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "ratios".into(),
            name: "Ratios Ltd".into(),
            admin_email: "admin@ratios.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["metrics".into(), "parties".into()],
            settings: Some(json!({"timezone": "UTC"})),
        })
        .await
        .expect("tenant");
    let session = k.login("ratios", "admin@ratios.test", "Correct-Horse-9").await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let rule = |p: Value| serde_json::from_value::<ferrum_meta::PermissionDef>(p).unwrap();
    k.set_role_permission(&admin, "Sales Rep", &rule(json!({"entity": "Customer", "read": true, "create": true, "if_owner": true}))).await.unwrap();
    k.set_role_permission(&admin, "Sales Director", &rule(json!({"entity": "Customer", "read": true, "create": true}))).await.unwrap();
    let user = |email: &'static str, roles: &'static [&'static str]| {
        let (k, admin) = (k.clone(), admin.clone());
        async move {
            let roles: Vec<String> = roles.iter().map(|r| r.to_string()).collect();
            let id = k.create_user(&admin, email, email, Some("Correct-Horse-9"), &roles, false).await.expect("user");
            k.load_ctx(tenant, id, None, "it".into()).await.expect("ctx")
        }
    };
    let author = user("analyst@ratios.test", &["Metric Author", "Sales Director"]).await;
    let steward = user("steward@ratios.test", &["Data Steward", "Sales Director"]).await;
    let director = user("director@ratios.test", &["Metric Reader", "Sales Director"]).await;
    let rep1 = user("rep1@ratios.test", &["Metric Reader", "Sales Rep"]).await;
    let rep2 = user("rep2@ratios.test", &["Metric Reader", "Sales Rep"]).await;
    let newcomer = user("new@ratios.test", &["Metric Reader", "Sales Rep"]).await;
    let customer = |name: &str, kind: &str, days: i64| obj(json!({"customer_name": name, "customer_type": kind, "payment_terms_days": days}));
    k.insert(&rep1, "Customer", customer("Acme", "Company", 30)).await.unwrap();
    k.insert(&rep1, "Customer", customer("Globex", "Company", 60)).await.unwrap();
    k.insert(&rep1, "Customer", customer("Ann", "Individual", 0)).await.unwrap();
    k.insert(&rep2, "Customer", customer("Initech", "Company", 90)).await.unwrap();

    let make = |name: &str, body: Value| {
        let (k, author) = (k.clone(), author.clone());
        let mut doc = body.as_object().cloned().unwrap();
        doc.insert("metric_name".into(), json!(name));
        async move { k.insert(&author, "Metric", doc).await }
    };
    let act = |id: Uuid, action: &str, params: Value| {
        let (k, steward, action) = (k.clone(), steward.clone(), action.to_string());
        async move { k.run_action(&steward, "Metric", id, &action, params).await }
    };
    let companies = json!([["customer_type", "eq", "Company"]]);
    let count = make(
        "Company customers",
        json!({"definition": {"entity": "Customer", "filters": companies, "measures": [{"fn": "count"}]}, "series_by": "created_at"}),
    )
    .await
    .unwrap();
    let terms = make("Company terms", json!({"definition": {"entity": "Customer", "filters": companies, "measures": [{"fn": "sum", "field": "payment_terms_days"}]}, "series_by": "created_at", "unit": "days"})).await.unwrap();
    let average = make("Average company terms", json!({"formula": "[Company terms] / [Company customers]", "unit": "days"})).await.unwrap();
    assert_eq!(average["uses"], "Composed of: Company customers, Company terms");

    // Made of others, never of itself, never both a query and a formula, never of what is not there.
    refused(make("Both", json!({"definition": {"entity": "Customer", "measures": [{"fn": "count"}]}, "formula": "[Company terms]"})).await, "not both");
    refused(make("Dangling", json!({"formula": "[No such metric] * 2"})).await, "there is no metric called No such metric");
    let loop_a = make("Loop A", json!({"formula": "[Company customers] * 1"})).await.unwrap();
    make("Loop B", json!({"formula": "[Loop A] + 1"})).await.unwrap();
    refused(k.update(&author, "Metric", uid(&loop_a["id"]), obj(json!({"formula": "[Loop B] - 1"})), None).await, "Loop A → Loop B → Loop A");

    // Certified after its parts, by a steward who does not own it.
    refused(act(uid(&average["id"]), "certify", Value::Null).await, "is certified first");
    act(uid(&count["id"]), "certify", Value::Null).await.unwrap();
    act(uid(&terms["id"]), "certify", Value::Null).await.unwrap();
    act(uid(&average["id"]), "certify", Value::Null).await.unwrap();

    // Each reads it from their own values of its parts: (30 + 60 + 90) / 3, (30 + 60) / 2, 90 / 1.
    assert_eq!(value(&k, &director, "Average company terms").await.unwrap(), 60.0);
    assert_eq!(value(&k, &rep1, "Average company terms").await.unwrap(), 45.0);
    assert_eq!(value(&k, &rep2, "Average company terms").await.unwrap(), 90.0);
    // Someone with no company customers of their own: no value, and said why, never zero.
    refused(value(&k, &newcomer, "Average company terms").await, "has no value now");

    // What a change to a part reaches.
    let impact = ask(&k, &steward, "/impact", json!({"metric": "Company customers"})).await.unwrap();
    assert_eq!(impact["used_by"], json!(["Average company terms", "Loop A", "Loop B"]));

    // Period by period: this month is not over, so it is partial; the months before had none.
    let series = ask(&k, &director, "/series", json!({"metric": "Average company terms", "bucket": "month", "periods": "3"})).await.unwrap();
    let points = series["points"].as_array().unwrap();
    assert_eq!(points.len(), 3, "{series}");
    assert_eq!((points[2]["value"].as_str(), points[2]["partial"].as_bool()), (Some("60"), Some(true)), "{series}");
    assert!(points[..2].iter().all(|p| p["value"].is_null() && p["partial"] == json!(false)), "no customers then, so no average: {series}");
    let counted = ask(&k, &rep1, "/series", json!({"metric": "Company customers", "bucket": "week", "periods": "2"})).await.unwrap();
    assert_eq!(counted["points"].as_array().unwrap().last().unwrap()["value"], "2", "{counted}");
    refused(ask(&k, &director, "/series", json!({"metric": "Company customers", "periods": "500"})).await, "1 to 120");

    // A part deprecated: the composed metric says so rather than meaning something else.
    act(uid(&terms["id"]), "deprecate", json!({"replaced_by": count["id"]})).await.unwrap();
    refused(value(&k, &director, "Average company terms").await, "Company terms is not certified");
}
