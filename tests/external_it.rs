//! Metrics read from external datasets through Metrics' engine (Ferrum's Logic Platform: FLC-SEM-005,
//! -006, -007, -010): the engine runs beside the kernel over HTTP, as it does beside the server, and every
//! expected number was worked out by hand from the dataset written here.
//!
//! | id | region | owner | status | total | secret |
//! |---|---|---|---|---|---|
//! | 1 | North | ann@x.test | Paid | 100.00 | a |
//! | 2 | North | bob@x.test | Open | 50.00 | b |
//! | 3 | South | Ann@X.test | Paid | 25.50 | c |
//! | 4 | South | cid@x.test | Paid | 10.00 | d |
//!
//! All: 185.5. North: 150. South: 35.5. Ann's, by address in any case: 125.5. Per customer (two of them),
//! for a reader of the North: 75. Against a real PostgreSQL (`FERRUM_METRICS_TEST_DATABASE_URL`).

use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings};
use ferrum_meta::Catalog;
use ferrum_metrics_engine::{Config, Dataset, Engine, Format, Limits};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

const TOKEN: &str = "external-it-0123456789abcdef0123456789";

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

/// A metric's value as its reader reads it, exactly as written, and what of it is stale.
async fn value(k: &Kernel, who: &Ctx, metric: &str) -> Result<(String, Vec<String>), Error> {
    let v = ask(k, who, "/value", json!({"metric": metric})).await?;
    let stale = v["stale"].as_array().unwrap().iter().map(|s| s["source"].as_str().unwrap().to_string()).collect();
    Ok((v["value"].as_str().unwrap().to_string(), stale))
}

fn refused<V: std::fmt::Debug>(r: Result<V, Error>, says: &str) {
    let msg = format!("{r:?}");
    assert!(r.is_err() && msg.contains(says), "expected a refusal saying {says:?}, got {msg}");
}

fn lake() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ferrum-external-it-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("orders.csv"),
        "id,region,owner,status,total,secret\n1,North,ann@x.test,Paid,100.00,a\n2,North,bob@x.test,Open,50.00,b\n3,South,Ann@X.test,Paid,25.50,c\n4,South,cid@x.test,Paid,10.00,d\n",
    )
    .unwrap();
    dir
}

#[test]
fn external_metrics_through_the_engine() {
    let dir = lake();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // SAFETY: set before the runtime starts a thread, and nothing else in this test binary reads them.
    unsafe {
        std::env::set_var("FERRUM_METRICS_ENGINE_URL", format!("http://127.0.0.1:{port}"));
        std::env::set_var("FERRUM_METRICS_ENGINE_TOKEN", TOKEN);
    }
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario(dir, listener)).await.unwrap() });
}

async fn scenario(dir: PathBuf, listener: std::net::TcpListener) {
    // ───────────── the engine, beside the kernel ─────────────
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let csv = |name: &str| Dataset { name: name.into(), workspace: "lake".into(), format: Format::Csv, path: dir.join("orders.csv") };
    let engine = Arc::new(Engine::new(Config { datasets: vec![csv("orders_lake")], limits: Limits::default() }).unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(ferrum_metrics_engine::serve(listener, engine, TOKEN.into(), async {
        let _ = stopped.await;
    }));

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
            slug: "lake".into(),
            name: "Lake Ltd".into(),
            admin_email: "admin@lake.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["metrics".into(), "parties".into()],
            settings: Some(json!({"timezone": "UTC"})),
        })
        .await
        .expect("tenant");
    let session = k.login("lake", "admin@lake.test", "Correct-Horse-9").await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let rule = |p: Value| serde_json::from_value::<ferrum_meta::PermissionDef>(p).unwrap();
    for role in ["North Sales", "South Sales", "Auditor"] {
        k.set_role_permission(&admin, role, &rule(json!({"entity": "Customer", "read": true}))).await.unwrap();
    }
    let user = |email: &'static str, roles: &'static [&'static str]| {
        let (k, admin) = (k.clone(), admin.clone());
        async move {
            let roles: Vec<String> = roles.iter().map(|r| r.to_string()).collect();
            let id = k.create_user(&admin, email, email, Some("Correct-Horse-9"), &roles, false).await.expect("user");
            k.load_ctx(tenant, id, None, "it".into()).await.expect("ctx")
        }
    };
    let author = user("analyst@lake.test", &["Metric Author", "North Sales"]).await;
    let auditor_author = user("auditor@lake.test", &["Metric Author", "Auditor"]).await;
    let steward = user("steward@lake.test", &["Data Steward", "North Sales"]).await;
    let north = user("north@lake.test", &["Metric Reader", "North Sales"]).await;
    let south = user("south@lake.test", &["Metric Reader", "South Sales"]).await;
    let regionless = user("nobody@lake.test", &["Metric Reader", "Auditor"]).await;
    let ann = user("ann@x.test", &["Metric Reader", "Auditor"]).await;

    // ───────────── the dataset, described by a steward ─────────────
    let source = |body: Value| {
        let (k, steward) = (k.clone(), steward.clone());
        async move { k.insert(&steward, "ExternalSource", obj(body)).await }
    };
    refused(source(json!({"source_name": "Orders"})).await, "lower-case letters");
    refused(source(json!({"source_name": "bad_rules", "rows_by": {"column": "region", "by": "team"}})).await, "by is email or roles");
    refused(source(json!({"source_name": "bad_columns", "hidden_columns": {"secret": "Auditor"}})).await, "{column: [role");
    let orders_source = source(json!({
        "source_name": "orders_lake", "fresh_within_minutes": 60,
        "rows_by": {"column": "region", "by": "roles", "map": {"North Sales": ["North"], "South Sales": ["South"]}},
        "hidden_columns": {"secret": ["Auditor"]}
    }))
    .await
    .expect("a source");
    source(json!({"source_name": "missing_lake"})).await.expect("a source the engine does not list");
    refused(k.insert(&north, "ExternalSource", obj(json!({"source_name": "mine"}))).await, "Forbidden");

    // ───────────── external metrics, checked as they are saved ─────────────
    let model = |table: &str| {
        json!({"catalog": "wren", "schema": "public", "models": [{"name": "orders", "tableReference": {"table": table}, "primaryKey": "id", "columns": [
            {"name": "id", "type": "integer"}, {"name": "region", "type": "varchar"}, {"name": "owner", "type": "varchar"},
            {"name": "status", "type": "varchar"}, {"name": "total", "type": "double"}, {"name": "secret", "type": "varchar"}
        ]}]})
    };
    let external = |sql: &str| json!({"model": model("orders_lake"), "sql": sql});
    let make = |who: &Ctx, name: &str, body: Value| {
        let (k, who) = (k.clone(), who.clone());
        let mut doc = body.as_object().cloned().unwrap();
        doc.insert("metric_name".into(), json!(name));
        async move { k.insert(&who, "Metric", doc).await }
    };
    let certify = |id: Uuid| {
        let (k, steward) = (k.clone(), steward.clone());
        async move { k.run_action(&steward, "Metric", id, "certify", Value::Null).await.expect("certified") }
    };
    let revenue =
        make(&author, "Lake revenue", json!({"external": external("select sum(total) from orders"), "unit": "USD"})).await.expect("an external metric");
    assert_eq!(revenue["uses"], "External: orders_lake through Wren (models orders)");
    refused(make(&author, "Dropped", json!({"external": external("drop table orders")})).await, "only a single SELECT");
    refused(make(&author, "Raw", json!({"external": external("select sum(total) from orders_lake")})).await, "is not a model");
    refused(make(&author, "Words", json!({"external": external("select max(owner) from orders")})).await, "one number");
    refused(
        make(&author, "Undescribed", json!({"external": {"model": model("ghost_lake"), "sql": "select count(*) from orders"}})).await,
        "ghost_lake is not an external source here",
    );
    refused(
        make(&author, "Unlisted", json!({"external": {"model": model("missing_lake"), "sql": "select count(*) from orders"}})).await,
        "not a dataset of this workspace",
    );
    refused(make(&author, "Both", json!({"external": external("select count(*) from orders"), "formula": "[Lake revenue]"})).await, "external query alone");
    // A column hidden from its author is not theirs to define a metric over; an auditor may.
    refused(make(&author, "Secrets", json!({"external": external("select count(secret) from orders")})).await, "does not run");
    let secrets = make(&auditor_author, "Secrets", json!({"external": external("select count(secret) from orders")})).await.expect("an auditor's metric");
    certify(uid(&revenue["id"])).await;
    certify(uid(&secrets["id"])).await;

    // ───────────── each reader's own rows and columns ─────────────
    assert_eq!(value(&k, &north, "Lake revenue").await.unwrap(), ("150".to_string(), vec![]));
    assert_eq!(value(&k, &south, "Lake revenue").await.unwrap().0, "35.5");
    assert_eq!(value(&k, &admin, "Lake revenue").await.unwrap().0, "185.5");
    assert_eq!(value(&k, &regionless, "Lake revenue").await.unwrap().0, "0", "no region of theirs: no rows, never every row");
    refused(value(&k, &north, "Secrets").await, "no value");
    assert_eq!(value(&k, &regionless, "Secrets").await.unwrap().0, "0", "an auditor reads the column, of the rows that are theirs");
    // By address, whatever its case.
    k.update(&steward, "ExternalSource", uid(&orders_source["id"]), obj(json!({"rows_by": {"column": "owner", "by": "email"}})), None).await.unwrap();
    assert_eq!(value(&k, &ann, "Lake revenue").await.unwrap().0, "125.5");
    assert_eq!(value(&k, &north, "Lake revenue").await.unwrap().0, "0");
    k.update(
        &steward,
        "ExternalSource",
        uid(&orders_source["id"]),
        obj(json!({"rows_by": {"column": "region", "by": "roles", "map": {"North Sales": ["North"], "South Sales": ["South"]}}})),
        None,
    )
    .await
    .unwrap();

    // ───────────── federated with the records, and fresh or stale ─────────────
    for name in ["Acme", "Globex"] {
        k.insert(&admin, "Customer", obj(json!({"customer_name": name, "customer_type": "Company"}))).await.unwrap();
    }
    let customers = make(&author, "Customers", json!({"definition": {"entity": "Customer", "measures": [{"fn": "count"}]}})).await.unwrap();
    certify(uid(&customers["id"])).await;
    let per = make(&author, "Revenue per customer", json!({"formula": "[Lake revenue] / [Customers]"})).await.unwrap();
    certify(uid(&per["id"])).await;
    assert_eq!(value(&k, &north, "Revenue per customer").await.unwrap(), ("75".to_string(), vec![]));
    let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    std::fs::File::options().write(true).open(dir.join("orders.csv")).unwrap().set_modified(two_hours_ago).unwrap();
    assert_eq!(value(&k, &north, "Lake revenue").await.unwrap(), ("150".to_string(), vec!["orders_lake".to_string()]), "older than its hour: said so");
    assert_eq!(value(&k, &north, "Revenue per customer").await.unwrap().1, vec!["orders_lake".to_string()], "and so is what is made of it");
    std::fs::File::options().write(true).open(dir.join("orders.csv")).unwrap().set_modified(std::time::SystemTime::now()).unwrap();
    assert_eq!(value(&k, &north, "Lake revenue").await.unwrap().1, Vec::<String>::new());
    refused(ask(&k, &north, "/series", json!({"metric": "Lake revenue"})).await, "has no series yet");

    // ───────────── a kept value says what it meant; a changed query is a draft again ─────────────
    let kept = k.run_action(&north, "Metric", uid(&revenue["id"]), "snapshot", Value::Null).await.expect("kept");
    assert_eq!(kept["value"], "150");
    // Its words may change freely; its query may not.
    k.update(&author, "Metric", uid(&revenue["id"]), obj(json!({"description": "Orders in the lake, by total"})), None).await.unwrap();
    assert_eq!(k.get(&admin, "Metric", uid(&revenue["id"]), false).await.unwrap()["status"], "Certified", "a description is not its definition");
    k.update(&author, "Metric", uid(&revenue["id"]), obj(json!({"external": external("select sum(total) from orders where status = 'Paid'")})), None)
        .await
        .unwrap();
    let changed = k.get(&admin, "Metric", uid(&revenue["id"]), false).await.unwrap();
    assert_eq!((changed["status"].as_str(), changed["certified_hash"].as_str()), (Some("Draft"), None), "{changed:?}");
    refused(value(&k, &north, "Lake revenue").await, "NotFound");
    certify(uid(&revenue["id"])).await;
    assert_eq!(value(&k, &north, "Lake revenue").await.unwrap().0, "100", "the North's paid orders: one of 100");

    // ───────────── the engine gone: no value, said so ─────────────
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
    refused(value(&k, &north, "Lake revenue").await, "cannot be reached");
    let gone = format!("{:?}", value(&k, &north, "Lake revenue").await);
    assert!(!gone.contains("127.0.0.1") && !gone.contains("http"), "the engine's address is not the reader's: {gone}");
    refused(value(&k, &north, "Revenue per customer").await, "cannot be reached");
}
