//! Governed datasets and the warehouse feed (FERRUM-ECOSYSTEM-010) against a real PostgreSQL
//! (`FERRUM_METRICS_DATASETS_TEST_DATABASE_URL`, default
//! `postgres://ferrum:ferrum@localhost:5432/ferrum_metrics_datasets_test`; reset first): a dataset
//! defined once across a linked record, refused when its columns would change with its rows or it
//! names none; its lineage naming the linked record; tried by its owner in draft; certified only by
//! a steward, keeping its columns; loaded as CSV and JSON lines by each reader with their own
//! rights; back in draft when its definition changes; deprecated, naming what to load instead; and
//! metrics' kept values loaded since a time, oldest first.

use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings, ListQuery};
use ferrum_meta::Catalog;
use serde_json::{Map, Value, json};
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

/// A Metrics endpoint as `who`: the body and the headers.
async fn feed(k: &Kernel, who: &Ctx, path: &str, query: Value) -> Result<(String, Vec<(String, String)>), Error> {
    let site = k.hooks.site("metrics").expect("metrics' endpoints").clone();
    let req = ferrum_kernel::SiteRequest {
        tenant: who.tenant,
        user: Some(who.clone()),
        method: "GET".into(),
        path: format!("/{path}"),
        query: query.as_object().cloned().unwrap_or_default(),
        headers: Default::default(),
        body: vec![],
        client_ip: "203.0.113.9".into(),
    };
    let r = site.handle(k, req).await?;
    Ok((String::from_utf8(r.body).unwrap(), r.headers))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

/// JSON lines, each by the columns certified: (group, count, average) as read.
fn by_group(body: &str, names: &[String]) -> Vec<(String, f64, f64)> {
    let mut out: Vec<(String, f64, f64)> = body
        .lines()
        .map(|l| {
            let row: Map<String, Value> = serde_json::from_str(l).unwrap();
            (row[&names[0]].as_str().unwrap_or_default().to_string(), dec(&row[&names[1]]), dec(&row[&names[2]]))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
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
fn governed_datasets() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_METRICS_DATASETS_TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_metrics_datasets_test".into());
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
            slug: "tables".into(),
            name: "Tables Inc".into(),
            admin_email: "admin@tables.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["metrics".into(), "parties".into()],
            settings: Some(json!({"timezone": "UTC"})),
        })
        .await
        .expect("tenant");
    let session = k.login("tables", "admin@tables.test", "Correct-Horse-9").await.expect("login");
    let a = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let user = |email: &'static str, roles: &'static [&'static str]| {
        let (k, a) = (k.clone(), a.clone());
        async move {
            let roles: Vec<String> = roles.iter().map(|r| r.to_string()).collect();
            let id = k.create_user(&a, email, email, Some("Correct-Horse-9"), &roles, false).await.expect("user");
            let s = k.login("tables", email, "Correct-Horse-9").await.expect("login");
            k.load_ctx(tenant, id, Some(s.token_version), "it".into()).await.expect("ctx")
        }
    };
    // Sales reps see the customers they made; their director sees them all; both see the groups.
    let rule = |p: Value| serde_json::from_value::<ferrum_meta::PermissionDef>(p).unwrap();
    k.set_role_permission(&a, "Sales Rep", &rule(json!({"entity": "Customer", "read": true, "create": true, "if_owner": true}))).await.unwrap();
    k.set_role_permission(&a, "Sales Director", &rule(json!({"entity": "Customer", "read": true, "create": true}))).await.unwrap();
    for role in ["Sales Rep", "Sales Director"] {
        k.set_role_permission(&a, role, &rule(json!({"entity": "CustomerGroup", "read": true}))).await.unwrap();
    }
    let author = user("analyst@tables.test", &["Metric Author", "Sales Director"]).await;
    let steward = user("steward@tables.test", &["Data Steward", "Sales Director"]).await;
    let rep1 = user("rep1@tables.test", &["Metric Reader", "Sales Rep"]).await;
    let rep2 = user("rep2@tables.test", &["Metric Reader", "Sales Rep"]).await;
    let director = user("director@tables.test", &["Metric Reader", "Sales Director"]).await;
    let group = |name: &str| {
        let (k, a) = (k.clone(), a.clone());
        let name = name.to_string();
        async move { uid(&k.insert(&a, "CustomerGroup", obj(json!({"group_name": name}))).await.unwrap()["id"]) }
    };
    let (retail, wholesale) = (group("Retail").await, group("Wholesale").await);
    let customer = |name: &str, g: Uuid, days: i64| json!({"customer_name": name, "customer_type": "Company", "customer_group": g.to_string(), "payment_terms_days": days});
    for (who, c) in [
        (&rep1, customer("Acme", wholesale, 30)),
        (&rep1, customer("Globex", wholesale, 60)),
        (&rep1, customer("Ann's", retail, 0)),
        (&rep2, customer("Initech", wholesale, 90)),
    ] {
        k.insert(who, "Customer", obj(c)).await.expect("customer");
    }

    // ───────────── a dataset across a linked record ─────────────
    let by_groups = json!({"entity": "Customer", "group_by": [{"field": "customer_group.group_name", "label": "Group"}],
                           "measures": [{"fn": "count", "label": "Customers"}, {"fn": "avg", "field": "payment_terms_days", "label": "Average Terms"}]});
    let set = |name: &str, definition: Value| json!({"dataset_name": name, "domain": "Sales", "definition": definition});
    refused(
        k.insert(&author, "Dataset", obj(set("Pivoted", json!({"entity": "Customer", "pivot": {"field": "customer_type"}, "measures": [{"fn": "count"}]}))))
            .await,
        "invalid",
        "no pivot or comparison",
    );
    refused(k.insert(&author, "Dataset", obj(set("Nothing", json!({"entity": "Customer"})))).await, "invalid", "names its columns");
    refused(
        k.insert(&author, "Dataset", obj(set("Bad", json!({"entity": "Customer", "fields": [{"field": "no_such_field"}]})))).await,
        "invalid",
        "does not run",
    );
    let d = k.insert(&author, "Dataset", obj(set("Customers by group", by_groups.clone()))).await.expect("a dataset");
    assert_eq!(
        (d["owner"].as_str(), d["uses"].as_str(), d["status"].as_str()),
        (Some("analyst@tables.test"), Some("Customer: payment_terms_days; CustomerGroup (by customer_group): group_name"), Some("Draft")),
        "its lineage names the linked record"
    );
    let did = uid(&d["id"]);
    let load = |who: &Ctx, format: &str| {
        let (k, who, format) = (k.clone(), who.clone(), format.to_string());
        async move { feed(&k, &who, "dataset", json!({"name": "Customers by group", "format": format})).await }
    };
    let (_, h) = load(&author, "csv").await.expect("its owner tries the draft");
    assert_eq!(header(&h, "x-ferrum-dataset-hash"), Some("draft"));
    refused(load(&rep1, "csv").await, "not found", "");

    // ───────────── certified by a steward, keeping its columns ─────────────
    refused(k.run_action(&author, "Dataset", did, "certify", Value::Null).await, "forbidden", "a Data Steward certifies a dataset");
    k.run_action(&steward, "Dataset", did, "certify", Value::Null).await.expect("certified");
    let c = k.get(&a, "Dataset", did, false).await.unwrap();
    let names: Vec<String> = c["columns"].as_str().unwrap().lines().map(|l| l.split([' ', ':']).next().unwrap().to_string()).collect();
    assert_eq!((c["status"].as_str(), names.len()), (Some("Certified"), 3), "{c:?}");
    let hash = c["certified_hash"].as_str().unwrap().to_string();

    // ───────────── loaded by each reader with their own rights ─────────────
    let (body, h) = load(&director, "ndjson").await.expect("loaded");
    assert_eq!(by_group(&body, &names), vec![("Retail".into(), 1.0, 0.0), ("Wholesale".into(), 3.0, 60.0)]);
    assert_eq!((header(&h, "x-ferrum-dataset-hash"), header(&h, "x-ferrum-truncated")), (Some(hash.as_str()), Some("false")));
    let (body, _) = load(&rep1, "ndjson").await.unwrap();
    assert_eq!(by_group(&body, &names), vec![("Retail".into(), 1.0, 0.0), ("Wholesale".into(), 2.0, 45.0)], "the three she made");
    let (body, _) = load(&rep2, "ndjson").await.unwrap();
    assert_eq!(by_group(&body, &names), vec![("Wholesale".into(), 1.0, 90.0)]);
    let (csv, h) = load(&rep1, "csv").await.unwrap();
    let lines: Vec<&str> = csv.split("\r\n").filter(|l| !l.is_empty()).collect();
    assert_eq!((lines[0], lines.len()), (names.join(",").as_str(), 3), "a header of its columns, then its rows");
    assert_eq!(header(&h, "content-disposition"), Some("attachment; filename=\"Customers_by_group.csv\""));
    refused(load(&rep1, "xlsx").await, "invalid", "is csv or ndjson");
    refused(feed(&k, &rep1, "dataset", json!({})).await, "invalid", "name");

    // ───────────── change control ─────────────
    k.update(&author, "Dataset", did, obj(json!({"description": "Customers by their group"})), None).await.unwrap();
    assert_eq!(k.get(&a, "Dataset", did, false).await.unwrap()["status"], "Certified", "its description is not its definition");
    let narrower = json!({"entity": "Customer", "filters": [["payment_terms_days", ">", 10]], "group_by": [{"field": "customer_group.group_name"}],
                          "measures": [{"fn": "count"}, {"fn": "avg", "field": "payment_terms_days"}]});
    k.update(&author, "Dataset", did, obj(json!({"definition": narrower})), None).await.unwrap();
    let changed = k.get(&a, "Dataset", did, false).await.unwrap();
    assert_eq!((changed["status"].as_str(), changed["columns"].as_str()), (Some("Draft"), None), "a new definition is a draft again");
    refused(load(&director, "csv").await, "not found", "");
    let seen = k.list(&rep1, "Dataset", &ListQuery::default()).await.unwrap().rows;
    assert!(seen.is_empty(), "readers see certified datasets only");
    k.run_action(&steward, "Dataset", did, "certify", Value::Null).await.unwrap();

    // ───────────── deprecation ─────────────
    let plain = json!({"entity": "Customer", "fields": [{"field": "customer_name"}, {"field": "customer_group.group_name"}]});
    let all = uid(&k.insert(&author, "Dataset", obj(set("Customers", plain))).await.unwrap()["id"]);
    k.run_action(&steward, "Dataset", all, "certify", Value::Null).await.unwrap();
    refused(k.run_action(&steward, "Dataset", did, "deprecate", json!({"replaced_by": did.to_string()})).await, "invalid", "does not replace itself");
    k.run_action(&steward, "Dataset", did, "deprecate", json!({"replaced_by": all.to_string()})).await.unwrap();
    refused(load(&director, "csv").await, "invalid", "deprecated: load Customers instead");
    let (body, _) = feed(&k, &director, "dataset", json!({"name": "Customers", "format": "ndjson"})).await.unwrap();
    assert_eq!(body.lines().count(), 4, "a table of records, each a row");

    // ───────────── metrics' values since a time ─────────────
    let count = json!({"metric_name": "Customers counted", "definition": {"entity": "Customer", "measures": [{"fn": "count"}]}});
    let m = uid(&k.insert(&author, "Metric", obj(count)).await.unwrap()["id"]);
    k.run_action(&steward, "Metric", m, "certify", Value::Null).await.unwrap();
    k.run_action(&rep1, "Metric", m, "snapshot", Value::Null).await.unwrap();
    k.run_action(&director, "Metric", m, "snapshot", Value::Null).await.unwrap();
    let (body, h) = feed(&k, &steward, "values", json!({})).await.expect("values");
    let kept: Vec<Value> = body.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(
        kept.iter().map(|v| (v["metric"].as_str().unwrap(), dec(&v["value"]))).collect::<Vec<_>>(),
        vec![("Customers counted", 3.0), ("Customers counted", 4.0)]
    );
    assert!(kept.iter().all(|v| v.get("read_by").is_none()), "who read it does not leave");
    let first = kept[0]["read_at"].as_str().unwrap().to_string();
    assert_eq!(header(&h, "x-ferrum-next-since"), kept[1]["read_at"].as_str());
    let (body, _) = feed(&k, &steward, "values", json!({"since": first})).await.unwrap();
    assert_eq!(body.lines().count(), 1, "only what came after");
    let (body, _) = feed(&k, &rep1, "values", json!({})).await.unwrap();
    assert_eq!(body.lines().count(), 1, "a reader loads her own");
    refused(feed(&k, &steward, "values", json!({"since": "yesterday"})).await, "invalid", "is a time like");

    // ───────────── who may do what ─────────────
    refused(k.insert(&rep1, "Dataset", obj(set("Mine", by_groups))).await, "forbidden", "");
}
