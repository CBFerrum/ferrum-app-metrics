//! A Wren semantic model brought into Metrics (Ferrum's Logic Platform: FLC-SEM-001, FLC-SEM-010): every
//! measure reviewed against the metrics defined here, a conflicting revenue kept as it is, what has no
//! exact meaning refused with its reason, and only what a reviewer accepts created, as drafts certified
//! like any other. Against a real PostgreSQL (`FERRUM_METRICS_TEST_DATABASE_URL`).

use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, Filter, HookRegistry, Kernel, KernelSettings};
use ferrum_meta::Catalog;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

const STORE: &[(&str, &str, &str)] = &[
    (
        "store",
        "package.yaml",
        r#"package: store
version: 0.1.0
label: Store
depends: [core]
modules:
  - name: store
    label: Store
    icon: store
    order: 91
    items: [Order]
roles:
  - name: Store Staff
    description: Sees every order.
    permissions:
      - {entity: Order, read: true, create: true, write: true}
"#,
    ),
    (
        "store",
        "entities/order.yaml",
        r#"- name: Order
  label: Order
  kind: master
  module: store
  fields:
    - {name: total, type: decimal, required: true}
    - {name: placed_on, type: date, required: true}
    - {name: buyer, type: data}
    - {name: status, type: select, options: [Open, Paid], default: Open}
"#,
    ),
];

/// A Wren manifest as its MDL schema has it: one model over orders, with a renamed column, a calculated
/// one, a relationship and a row rule; a cube of measures some of which have no exact meaning here; and
/// a cube over a view.
fn manifest() -> Value {
    json!({
        "catalog": "shop", "schema": "public",
        "models": [{
            "name": "orders", "tableReference": {"table": "orders"}, "primaryKey": "id",
            "columns": [
                {"name": "id", "type": "uuid"},
                {"name": "order_total", "type": "decimal", "expression": "total"},
                {"name": "placed_on", "type": "date"},
                {"name": "buyer", "type": "varchar"},
                {"name": "status", "type": "varchar"},
                {"name": "doubled", "type": "decimal", "isCalculated": true, "expression": "total * 2"},
                {"name": "customer", "type": "customers", "relationship": "orders_customers"}
            ],
            "rowLevelAccessControls": [{"name": "own region", "requiredProperties": [], "condition": "region = @session_region"}]
        }],
        "cubes": [{
            "name": "sales", "baseObject": "orders",
            "measures": [
                {"name": "revenue", "expression": "sum(order_total)", "type": "decimal", "description": "Everything ordered"},
                {"name": "orders", "expression": "count(*)", "type": "bigint"},
                {"name": "buyers", "expression": "count(distinct buyer)", "type": "bigint"},
                {"name": "largest", "expression": "max(order_total)", "type": "decimal"},
                {"name": "paid_revenue", "expression": "sum(case when status = 'Paid' then order_total end)", "type": "decimal"},
                {"name": "doubled_revenue", "expression": "sum(doubled)", "type": "decimal"},
                {"name": "customers", "expression": "count(distinct customer)", "type": "bigint"},
                {"name": "median_order", "expression": "median(order_total)", "type": "decimal"}
            ],
            "dimensions": [{"name": "status", "expression": "status", "type": "varchar"}],
            "timeDimensions": [{"name": "placed", "expression": "placed_on", "type": "date"}]
        }, {
            "name": "by_view", "baseObject": "orders_view",
            "measures": [{"name": "rows", "expression": "count(*)", "type": "bigint"}]
        }]
    })
}

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

async fn site(k: &Kernel, who: &Ctx, method: &str, path: &str, query: Value, body: Value) -> Result<Value, Error> {
    let site = k.hooks.site("metrics").expect("metrics' endpoint").clone();
    let req = ferrum_kernel::SiteRequest {
        tenant: who.tenant,
        user: Some(who.clone()),
        method: method.into(),
        path: path.into(),
        query: query.as_object().cloned().unwrap_or_default(),
        headers: Default::default(),
        body: if body.is_null() { vec![] } else { body.to_string().into_bytes() },
        client_ip: "203.0.113.9".into(),
    };
    let r = site.handle(k, req).await?;
    Ok(serde_json::from_slice(&r.body).unwrap())
}

async fn import(k: &Kernel, who: &Ctx, body: Value) -> Result<Value, Error> {
    site(k, who, "POST", "/import/wren", Value::Null, body).await
}

fn refused<V: std::fmt::Debug>(r: Result<V, Error>, says: &str) {
    let msg = format!("{r:?}");
    assert!(r.is_err() && msg.contains(says), "expected a refusal saying {says:?}, got {msg}");
}

/// The review's row for a measure.
fn row<'a>(review: &'a Value, measure: &str) -> &'a Value {
    review["review"].as_array().unwrap().iter().find(|r| r["measure"] == measure).unwrap_or_else(|| panic!("no row for {measure}: {review}"))
}

#[test]
fn a_wren_model_is_reviewed_never_a_second_authority() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_METRICS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_metrics_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 16, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_metrics::packages().expect("app packages"));
    packages.extend(ferrum_meta::catalog::load_embedded(STORE).expect("the store"));
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_metrics::register(&mut hooks);
    let settings = KernelSettings { secret_key: Some(SecretKey::new([7u8; 32])), ..Default::default() };
    let k: Arc<Kernel> = Kernel::boot(db, Catalog::from_packages(packages).expect("catalog"), hooks, settings, None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "wren".into(),
            name: "Wren Imports Ltd".into(),
            admin_email: "admin@wren.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["metrics".into(), "store".into()],
            settings: Some(json!({"timezone": "UTC"})),
        })
        .await
        .expect("tenant");
    let session = k.login("wren", "admin@wren.test", "Correct-Horse-9").await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let user = |email: &'static str, roles: &'static [&'static str]| {
        let (k, admin) = (k.clone(), admin.clone());
        async move {
            let roles: Vec<String> = roles.iter().map(|r| r.to_string()).collect();
            let id = k.create_user(&admin, email, email, Some("Correct-Horse-9"), &roles, false).await.expect("user");
            k.load_ctx(tenant, id, None, "it".into()).await.expect("ctx")
        }
    };
    let author = user("analyst@wren.test", &["Metric Author", "Store Staff"]).await;
    let steward = user("steward@wren.test", &["Data Steward", "Store Staff"]).await;
    let reader = user("reader@wren.test", &["Metric Reader", "Store Staff"]).await;
    for (total, status, buyer) in [("100", "Paid", "Ann"), ("50", "Open", "Bob"), ("25.5", "Paid", "Ann")] {
        k.insert(&author, "Order", obj(json!({"total": total, "status": status, "buyer": buyer, "placed_on": "2026-10-01"}))).await.unwrap();
    }

    // What Metrics already defines: revenue as paid orders only, certified; the order count and the
    // largest order under names of their own.
    let make = |name: &str, definition: Value| {
        let (k, author) = (k.clone(), author.clone());
        let doc = obj(json!({"metric_name": name, "definition": definition}));
        async move { k.insert(&author, "Metric", doc).await.expect("a metric") }
    };
    let id_of = |m: &Doc| m["id"].as_str().unwrap().parse::<Uuid>().unwrap();
    let revenue = make("Revenue", json!({"entity": "Order", "filters": [["status", "eq", "Paid"]], "measures": [{"fn": "sum", "field": "total"}]})).await;
    k.run_action(&steward, "Metric", id_of(&revenue), "certify", Value::Null).await.unwrap();
    let certified = k.get(&admin, "Metric", id_of(&revenue), false).await.unwrap();
    make("Order count", json!({"entity": "Order", "measures": [{"fn": "count"}]})).await;
    make("Largest", json!({"entity": "Order", "measures": [{"fn": "max", "field": "total"}]})).await;

    // ───────────── unmapped, nothing is guessed ─────────────
    let unmapped = import(&k, &author, json!({"mdl": manifest()})).await.unwrap();
    assert_eq!(row(&unmapped, "sales.revenue")["status"], "unsupported");
    assert!(row(&unmapped, "sales.revenue")["reason"].as_str().unwrap().contains("orders is not mapped to a record: map it to one"), "{unmapped}");
    assert!(unmapped["created"].as_array().unwrap().is_empty());

    // ───────────── mapped: the review ─────────────
    let map = json!({"orders": {"entity": "Order"}});
    let review = import(&k, &author, json!({"mdl": manifest(), "map": map})).await.unwrap();
    let status = |m: &str| row(&review, m)["status"].as_str().unwrap().to_string();
    let reason = |m: &str| row(&review, m)["reason"].as_str().unwrap_or_default().to_string();
    // A revenue that means something else: a conflict, said so, and the certified one is left alone.
    assert_eq!(status("sales.revenue"), "conflict");
    assert_eq!(row(&review, "sales.revenue")["existing"], "Revenue");
    assert!(reason("sales.revenue").contains("it stays as it is"), "{review}");
    // Wren's renamed column is the record's field it reads.
    assert_eq!(row(&review, "sales.revenue")["definition"], json!({"entity": "Order", "measures": [{"fn": "sum", "field": "total"}]}));
    assert_eq!(row(&review, "sales.revenue")["series_by"], "placed_on");
    assert_eq!((status("sales.orders"), row(&review, "sales.orders")["existing"].as_str()), ("duplicate".into(), Some("Order count")));
    assert_eq!((status("sales.largest"), row(&review, "sales.largest")["existing"].as_str()), ("same".into(), Some("Largest")));
    assert_eq!(status("sales.buyers"), "new");
    assert_eq!(row(&review, "sales.buyers")["definition"]["measures"][0], json!({"fn": "count_distinct", "field": "buyer"}));
    // No exact meaning here, so no approximation: each refused with its reason.
    for (m, says) in [
        ("sales.paid_revenue", "has no exact meaning here"),
        ("sales.median_order", "has no exact meaning here"),
        ("sales.doubled_revenue", "doubled is calculated in Wren (total * 2)"),
        ("sales.customers", "customer is a relationship in Wren"),
        ("by_view.rows", "orders_view is not a model"),
    ] {
        assert_eq!(status(m), "unsupported", "{m}: {review}");
        assert!(reason(m).contains(says), "{m} says {:?}", reason(m));
    }
    let notes = row(&review, "sales.buyers")["notes"].to_string();
    assert!(notes.contains("row and column rules on orders are not brought") && notes.contains("dimensions are not brought"), "{notes}");
    assert!(review["created"].as_array().unwrap().is_empty(), "a review creates nothing");

    // ───────────── accepting: all or nothing ─────────────
    let accept = |list: Value| json!({"mdl": manifest(), "map": map, "accept": list});
    refused(import(&k, &author, accept(json!([{"measure": "sales.buyers"}, {"measure": "sales.revenue"}]))).await, "import it under another name or leave it");
    let named = |name: &'static str| {
        let (k, admin) = (k.clone(), admin.clone());
        async move {
            let mut tx = k.db.tenant_tx(admin.tenant).await.unwrap();
            let found = k.find_tx(&admin, &mut tx, "Metric", &[Filter::eq("metric_name", name)], None, 1).await.unwrap();
            tx.commit().await.unwrap();
            found.into_iter().next()
        }
    };
    assert!(named("buyers").await.is_none(), "one refused acceptance creates nothing");
    refused(import(&k, &author, accept(json!([{"measure": "sales.paid_revenue"}]))).await, "sales.paid_revenue is not imported (unsupported)");
    refused(import(&k, &author, accept(json!([{"measure": "sales.orders"}]))).await, "sales.orders is not imported (duplicate)");
    refused(import(&k, &author, accept(json!([{"measure": "sales.largest"}]))).await, "sales.largest is not imported (same)");
    refused(import(&k, &author, accept(json!([{"measure": "sales.revenue", "as": "revenue"}]))).await, "there is already a metric called revenue");
    refused(import(&k, &author, accept(json!([{"measure": "sales.nothing"}]))).await, "there is no measure sales.nothing");
    refused(import(&k, &author, accept(json!([{"measure": "sales.buyers", "overwrite": true}]))).await, "overwrite is not something an acceptance takes");
    refused(import(&k, &author, json!({"mdl": manifest(), "replace": true})).await, r#"field: "replace", message: "is not something an import takes"#);
    refused(import(&k, &reader, json!({"mdl": manifest()})).await, "a Metric Author imports metrics");
    refused(import(&k, &author, json!({"mdl": {"cubes": []}})).await, "has no cubes");

    let done = import(&k, &author, accept(json!([{"measure": "sales.buyers"}, {"measure": "sales.revenue", "as": "Revenue (Wren)"}]))).await.unwrap();
    let created = done["created"].as_array().unwrap();
    assert_eq!(created.len(), 2, "{done}");
    assert!(created.iter().all(|c| c["status"] == "Draft"), "drafts, to be certified: {done}");
    let wren = named("Revenue (Wren)").await.expect("the import, under its own name");
    assert_eq!(
        (wren["owner"].as_str(), wren["uses"].as_str(), wren["series_by"].as_str()),
        (Some("analyst@wren.test"), Some("Order: total"), Some("placed_on"))
    );
    assert!(wren["description"].as_str().unwrap().starts_with("Everything ordered. Imported from Wren: cube sales, measure revenue = sum(order_total)"));
    // The certified revenue is exactly as it was.
    let still = k.get(&admin, "Metric", id_of(&revenue), false).await.unwrap();
    for f in ["status", "certified_hash", "certified_by", "definition"] {
        assert_eq!(still[f], certified[f], "{f}");
    }
    // Brought in once: the same measure again is the same metric now.
    let again = import(&k, &author, json!({"mdl": manifest(), "map": map})).await.unwrap();
    assert_eq!((row(&again, "sales.buyers")["status"].as_str(), row(&again, "sales.buyers")["existing"].as_str()), (Some("same"), Some("buyers")));

    // ───────────── then an ordinary metric ─────────────
    let read = |name: &str| {
        let (k, reader, name) = (k.clone(), reader.clone(), name.to_string());
        async move { site(&k, &reader, "GET", "/value", json!({"metric": name}), Value::Null).await }
    };
    refused(read("Revenue (Wren)").await, "NotFound");
    k.run_action(&steward, "Metric", wren["id"].as_str().unwrap().parse().unwrap(), "certify", Value::Null).await.expect("certified by a steward");
    // Two names, two meanings, side by side: paid orders, and every order.
    assert_eq!(read("Revenue").await.unwrap()["value"], "125.5");
    assert_eq!(read("Revenue (Wren)").await.unwrap()["value"], "175.5");
}
