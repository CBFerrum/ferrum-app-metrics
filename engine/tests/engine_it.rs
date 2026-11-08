//! Metrics' engine against datasets written here (Ferrum's Logic Platform: FLC-SEM-005, -006, -007, -010).
//! Every expected number was worked out by hand from these rows:
//!
//! | id | region | owner | status | total | secret |
//! |---|---|---|---|---|---|
//! | 1 | North | ann@x.test | Paid | 100.00 | a |
//! | 2 | North | bob@x.test | Open | 50.00 | b |
//! | 3 | South | Ann@X.test | Paid | 25.50 | c |
//! | 4 | South | cid@x.test | Paid | 10.00 | d |
//!
//! and regions North → Nora, South → Sam. All: 185.5. Paid: 135.5. North: 150. Ann's (any case): 125.5.
//! By manager, through Wren's relationship: Nora 150, Sam 35.5. Doubled, a calculated column: 371.

use ferrum_metrics_engine::{Allowed, Config, Dataset, Engine, Format, Limits, Query, RowRule};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A lake of its own for each test, so one test's changes are not another's.
fn lake(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ferrum-engine-it-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("orders.csv"),
        "id,region,owner,status,total,secret\n1,North,ann@x.test,Paid,100.00,a\n2,North,bob@x.test,Open,50.00,b\n3,South,Ann@X.test,Paid,25.50,c\n4,South,cid@x.test,Paid,10.00,d\n",
    )
    .unwrap();
    std::fs::write(dir.join("regions.csv"), "region,manager\nNorth,Nora\nSouth,Sam\n").unwrap();
    std::fs::write(dir.join("globex.csv"), "id,region,owner,status,total,secret\n9,North,zed@y.test,Paid,1000.00,z\n").unwrap();
    dir
}

fn engine(dir: &Path, limits: Limits) -> Engine {
    let d = |name: &str, workspace: &str, file: &str| Dataset { name: name.into(), workspace: workspace.into(), format: Format::Csv, path: dir.join(file) };
    let config = Config {
        datasets: vec![d("orders_lake", "acme", "orders.csv"), d("regions_lake", "acme", "regions.csv"), d("orders_lake", "globex", "globex.csv")],
        limits,
    };
    Engine::new(config).expect("an engine")
}

/// The semantic model: orders and regions, a relationship between them, and calculated columns.
fn model() -> Value {
    json!({
        "catalog": "wren", "schema": "public", "dataSource": "postgres",
        "models": [
            {"name": "orders", "tableReference": {"table": "orders_lake"}, "primaryKey": "id", "columns": [
                {"name": "id", "type": "integer"},
                {"name": "region", "type": "varchar"},
                {"name": "owner", "type": "varchar"},
                {"name": "status", "type": "varchar"},
                {"name": "total", "type": "double"},
                {"name": "secret", "type": "varchar"},
                {"name": "doubled", "type": "double", "isCalculated": true, "expression": "total * 2"},
                {"name": "leak", "type": "varchar", "isCalculated": true, "expression": "secret"},
                {"name": "region_of", "type": "regions", "relationship": "orders_regions"},
                {"name": "manager", "type": "varchar", "isCalculated": true, "expression": "region_of.manager"}
            ]},
            {"name": "regions", "tableReference": {"table": "regions_lake"}, "primaryKey": "region", "columns": [
                {"name": "region", "type": "varchar"},
                {"name": "manager", "type": "varchar"}
            ]}
        ],
        "relationships": [{"name": "orders_regions", "models": ["orders", "regions"], "joinType": "MANY_TO_ONE", "condition": "orders.region = regions.region"}]
    })
}

fn allow(name: &str) -> Allowed {
    Allowed { name: name.into(), rows: vec![], hide: vec![] }
}

fn everything() -> Vec<Allowed> {
    vec![allow("orders_lake"), allow("regions_lake")]
}

fn query(sql: &str, datasets: Vec<Allowed>) -> Query {
    Query { workspace: "acme".into(), tenant: "t-acme".into(), manifest: model(), sql: sql.into(), datasets }
}

/// The single value a query answers with.
async fn one(e: &Engine, sql: &str, datasets: Vec<Allowed>) -> Option<String> {
    let a = e.query(&query(sql, datasets)).await.unwrap_or_else(|r| panic!("{sql}: {r}"));
    assert_eq!((a.rows.len(), a.columns.len()), (1, 1), "{sql}: {a:?}");
    a.rows[0][0].clone()
}

async fn refused(e: &Engine, q: Query, status: u16, says: &str) {
    match e.query(&q).await {
        Ok(a) => panic!("{}: answered {a:?}, expected a refusal saying {says:?}", q.sql),
        Err(r) => assert!(r.status == status && r.message.contains(says), "{}: expected {status} saying {says:?}, got {r}", q.sql),
    }
}

#[tokio::test]
async fn supported_models_answer_exactly() {
    let e = engine(&lake("supported"), Limits::default());
    assert_eq!(one(&e, "select sum(total) from orders", everything()).await.as_deref(), Some("185.5"));
    assert_eq!(one(&e, "select sum(case when status = 'Paid' then total end) from orders", everything()).await.as_deref(), Some("135.5"));
    assert_eq!(one(&e, "select sum(doubled) from orders", everything()).await.as_deref(), Some("371.0"));
    assert_eq!(one(&e, "select count(*) from wren.public.orders where region = 'North'", everything()).await.as_deref(), Some("2"));
    // Through the relationship, as Wren plans it.
    let by_manager = e.query(&query("select manager, sum(total) from orders group by manager order by manager", everything())).await.unwrap();
    assert_eq!(by_manager.rows, vec![vec![Some("Nora".into()), Some("150.0".into())], vec![Some("Sam".into()), Some("35.5".into())]], "{by_manager:?}");
    assert!(by_manager.sql.contains("orders_lake") && by_manager.sql.contains("regions_lake"), "what ran names the datasets: {}", by_manager.sql);
    // A correlated question: answered exactly, or refused; never a wrong number.
    match e.query(&query("select id from orders o where total > (select avg(total) from orders i where i.region = o.region) order by id", everything())).await {
        Ok(a) => assert_eq!(a.rows, vec![vec![Some("1".into())], vec![Some("3".into())]], "{a:?}"),
        Err(r) => assert_eq!(r.status, 400, "{r}"),
    }
}

#[tokio::test]
async fn what_has_no_exact_meaning_fails_saying_why() {
    let e = engine(&lake("unsupported"), Limits::default());
    for (sql, says) in [
        ("select median_absolute(total) from orders", "does not plan"),
        ("select top 5 total from orders", "not implemented: TOP"),
        ("select ghost from orders", "does not plan"),
    ] {
        refused(&e, query(sql, everything()), 400, says).await;
    }
    // A model that reads SQL of its own, or a dataset in another schema, or one named as a model is.
    let mut by_sql = model();
    by_sql["models"][1]["refSql"] = json!("select 1");
    refused(&e, Query { manifest: by_sql, ..query("select sum(total) from orders", everything()) }, 400, "not SQL or another object").await;
    let mut elsewhere = model();
    elsewhere["models"][1]["tableReference"] = json!({"schema": "other", "table": "regions_lake"});
    refused(&e, Query { manifest: elsewhere, ..query("select sum(total) from orders", everything()) }, 400, "by its table alone").await;
    let mut same_name = model();
    same_name["models"][1]["tableReference"] = json!({"table": "orders"});
    refused(&e, Query { manifest: same_name, ..query("select sum(total) from orders", everything()) }, 400, "both called orders").await;
}

#[tokio::test]
async fn each_reader_sees_only_their_rows_and_columns() {
    let e = engine(&lake("rights"), Limits::default());
    let owned = |who: &str| {
        vec![
            Allowed { name: "orders_lake".into(), rows: vec![RowRule { column: "owner".into(), values: vec![who.into()], any_case: true }], hide: vec![] },
            allow("regions_lake"),
        ]
    };
    assert_eq!(one(&e, "select sum(total) from orders", owned("ann@x.test")).await.as_deref(), Some("125.5"), "rows 1 and 3, whatever the case");
    assert_eq!(one(&e, "select sum(total) from orders", owned("bob@x.test")).await.as_deref(), Some("50.0"));
    // An address is matched as itself: a wildcard in it matches nothing more.
    assert_eq!(one(&e, "select sum(total) from orders", owned("%@x.test")).await, None);
    let regions = |values: Vec<&str>| {
        vec![
            Allowed {
                name: "orders_lake".into(),
                rows: vec![RowRule { column: "region".into(), values: values.into_iter().map(String::from).collect(), any_case: false }],
                hide: vec![],
            },
            allow("regions_lake"),
        ]
    };
    assert_eq!(one(&e, "select sum(total) from orders", regions(vec!["North"])).await.as_deref(), Some("150.0"));
    assert_eq!(one(&e, "select sum(total) from orders", regions(vec![])).await, None, "no region of theirs: no rows, not every row");
    // Through the relationship too: only the reader's orders reach the managers.
    let a = e.query(&query("select manager, sum(total) from orders group by manager order by manager", regions(vec!["South"]))).await.unwrap();
    assert_eq!(a.rows, vec![vec![Some("Sam".into()), Some("35.5".into())]], "{a:?}");
    // A hidden column is not there for them: neither by its name nor through a calculated column.
    let hidden = vec![Allowed { name: "orders_lake".into(), rows: vec![], hide: vec!["secret".into()] }, allow("regions_lake")];
    for sql in ["select count(secret) from orders", "select count(leak) from orders"] {
        refused(&e, query(sql, hidden.clone()), 400, "").await;
    }
    // What a rule names must be there; and nothing is read that the rules do not say.
    refused(
        &e,
        query("select sum(total) from orders", vec![Allowed { name: "orders_lake".into(), rows: vec![], hide: vec!["nope".into()] }, allow("regions_lake")]),
        400,
        "no column nope",
    )
    .await;
    refused(&e, query("select sum(total) from orders", vec![allow("orders_lake")]), 400, "what this reader may see of regions_lake was not said").await;
}

#[tokio::test]
async fn hidden_columns_leave_the_rest_readable() {
    let e = engine(&lake("hidden-rest"), Limits::default());
    let hidden = vec![Allowed { name: "orders_lake".into(), rows: vec![], hide: vec!["secret".into()] }, allow("regions_lake")];
    let answer = e.query(&query("select sum(total) from orders", hidden)).await;
    // The model is read without the hidden column: what does not name it is answered.
    assert_eq!(answer.map(|a| a.rows), Ok(vec![vec![Some("185.5".into())]]));
}

#[tokio::test]
async fn only_a_select_of_the_models_runs() {
    let e = engine(&lake("sandbox"), Limits::default());
    for (sql, says) in [
        ("select * from orders_lake", "is not a model"),
        ("select * from '/etc/passwd'", "is not a model"),
        ("select sum(total) from orders, regions_lake", "is not a model"),
        ("create table copied as select * from orders", "only a single SELECT"),
        ("drop table orders", "only a single SELECT"),
        ("insert into orders values (5, 'North', 'x', 'Paid', 1, 'x')", "only a single SELECT"),
        ("set datafusion.execution.batch_size = 1", "only a single SELECT"),
        ("copy orders to '/tmp/out.csv'", "only a single SELECT"),
        ("select 1; select 2", "does not parse"),
    ] {
        refused(&e, query(sql, everything()), 400, says).await;
    }
    // Another workspace's dataset of the same name is its own.
    refused(
        &e,
        Query { workspace: "initech".into(), tenant: "t-initech".into(), ..query("select sum(total) from orders", everything()) },
        404,
        "not a dataset of this workspace",
    )
    .await;
    let theirs = Query { workspace: "globex".into(), tenant: "t-globex".into(), ..query("select sum(total) from orders", vec![allow("orders_lake")]) };
    let mut only_orders = model();
    only_orders["models"].as_array_mut().unwrap().truncate(1);
    only_orders["models"][0]["columns"].as_array_mut().unwrap().truncate(8);
    only_orders.as_object_mut().unwrap().remove("relationships");
    let a = e.query(&Query { manifest: only_orders, ..theirs }).await.unwrap();
    assert_eq!(a.rows, vec![vec![Some("1000.0".into())]]);
}

#[tokio::test]
async fn time_memory_and_rows_are_bounded() {
    let dir = lake("bounds");
    let few = engine(&dir, Limits { rows: 2, ..Limits::default() });
    let a = few.query(&query("select id from orders order by id", everything())).await.unwrap();
    assert_eq!((a.rows, a.truncated), (vec![vec![Some("1".into())], vec![Some("2".into())]], true));
    let small = engine(&dir, Limits { memory_mb: 1, ..Limits::default() });
    assert_eq!(one(&small, "select sum(total) from orders", everything()).await.as_deref(), Some("185.5"), "a small question fits");
    // A million groups out of ten copies of four orders: more than a megabyte, and nothing may spill.
    let explosive = "select a.id + 10 * b.id + 100 * c.id + 1000 * d.id + 10000 * e.id + 100000 * f.id + 1000000 * g.id + 10000000 * h.id + 100000000 * i.id + 1000000000 * j.id as k, count(*) \
        from orders a cross join orders b cross join orders c cross join orders d cross join orders e cross join orders f cross join orders g cross join orders h cross join orders i cross join orders j group by k";
    refused(&small, query(explosive, everything()), 422, "more memory than one query may hold").await;
    // Longer than its time, whether or not its work stops to yield: refused once its turn's wait and its time are
    // up, as the service runs it. Four orders crossed eleven times are four million rows to add up.
    let hasty = Arc::new(engine(&dir, Limits { millis: 100, wait_millis: 100, ..Limits::default() }));
    let terms = (0..11).map(|i| format!("o{i}.total")).collect::<Vec<_>>().join(" + ");
    let slow = format!("select sum({terms}) from orders o0{}", (1..11).map(|i| format!(" cross join orders o{i}")).collect::<String>());
    let started = std::time::Instant::now();
    match hasty.answer(query(&slow, everything())).await {
        Err(r) => assert!(r.status == 504 && r.message.contains("longer than 100 ms"), "{r}"),
        Ok(a) => panic!("answered after {:?}: {a:?}", started.elapsed()),
    }
    assert!(started.elapsed() < std::time::Duration::from_millis(1500), "refused after {:?}", started.elapsed());
}

#[tokio::test]
async fn a_dataset_that_cannot_be_read_says_so_without_its_files() {
    let dir = lake("unreadable");
    // Rows past the ones its columns' types are worked out from, with one total that is not a number.
    let mut rows = String::from("id,region,owner,status,total,secret\n");
    for i in 1..=1500 {
        rows.push_str(&format!("{i},North,ann@x.test,Paid,{},a\n", if i == 1400 { "lots".to_string() } else { "1.00".to_string() }));
    }
    std::fs::write(dir.join("orders.csv"), rows).unwrap();
    let e = engine(&dir, Limits::default());
    let place = dir.to_string_lossy().to_string();
    // The parser's own error carries the raw row, read before the reader's rules: none of it reaches the reader.
    let south = vec![
        Allowed {
            name: "orders_lake".into(),
            rows: vec![RowRule { column: "region".into(), values: vec!["South".into()], any_case: false }],
            hide: vec!["secret".into()],
        },
        allow("regions_lake"),
    ];
    match e.query(&query("select sum(total) from orders", south)).await {
        Err(r) => assert!(
            r.status == 503
                && r.message.contains("cannot be read now")
                && !r.message.contains(&place)
                && !r.message.contains("ann@x.test")
                && !r.message.contains("lots"),
            "{r}"
        ),
        Ok(a) => panic!("answered {a:?}"),
    }
    // A file it may not open at all.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("orders.csv"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let r = e.query(&query("select sum(total) from orders", everything())).await.expect_err("unreadable");
        std::fs::set_permissions(dir.join("orders.csv"), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(r.status == 503 && r.message.contains("cannot be read now") && !r.message.contains(&place), "{r}");
    }
}

#[tokio::test]
async fn answers_say_when_their_data_was_written() {
    let dir = lake("fresh");
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_767_323_045); // 2026-01-02T03:04:05Z
    std::fs::File::options().write(true).open(dir.join("orders.csv")).unwrap().set_modified(at).unwrap();
    let e = engine(&dir, Limits::default());
    let a = e.query(&query("select sum(total) from orders", everything())).await.unwrap();
    let orders = a.datasets.iter().find(|d| d.name == "orders_lake").unwrap();
    assert_eq!(orders.written_at.as_deref(), Some("2026-01-02T03:04:05Z"));
    // Gone, it cannot be read: said, not answered without it.
    std::fs::remove_file(dir.join("orders.csv")).unwrap();
    refused(&e, query("select sum(total) from orders", everything()), 503, "orders_lake cannot be read now").await;
    // Checked and not run: one numeric column.
    std::fs::write(dir.join("orders.csv"), "id,region,owner,status,total,secret\n1,North,ann@x.test,Paid,1,a\n").unwrap();
    let c = e.check(&query("select sum(total) from orders", everything())).await.unwrap();
    assert!(c.columns.len() == 1 && c.columns[0].numeric, "{c:?}");
    let words = e.check(&query("select max(owner) from orders", everything())).await.unwrap();
    assert!(!words.columns[0].numeric, "{words:?}");
}

/// `f` on a thread with the stack the engine's own threads have, as a query runs in the service: it ends
/// answered or refused, since a stack overflowed would take the whole process down.
fn on_an_engine_thread<F: Future<Output = ()>>(f: impl FnOnce() -> F + Send + 'static) {
    let thread = std::thread::Builder::new()
        .stack_size(ferrum_metrics_engine::STACK)
        .spawn(move || tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f()));
    thread.unwrap().join().expect("answered or refused");
}

#[test]
fn a_deep_query_is_answered_or_refused_never_the_engine_lost() {
    on_an_engine_thread(|| async {
        let e = engine(&lake("deep"), Limits::default());
        // Long and shallow is answered: 2,000 values in one list.
        let ids = (1..=2000).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        assert_eq!(one(&e, &format!("select sum(total) from orders where id in ({ids})"), everything()).await.as_deref(), Some("185.5"));
        // Terms nested as deep as a metric could need are answered: 185.5 × 90 = 16,695.
        let chain = |n: usize| format!("select sum({}) from orders", vec!["total"; n].join(" + "));
        assert_eq!(one(&e, &chain(90), everything()).await.as_deref(), Some("16695.0"));
        // Deeper is refused before anything is planned, even the deepest a query of the largest size can be.
        refused(&e, query(&chain(101), everything()), 400, "terms nest at most 100 deep").await;
        let deepest = format!("select sum(total{}) from orders", "+1".repeat(9985));
        assert!(deepest.len() <= 20_000, "{}", deepest.len());
        refused(&e, query(&deepest, everything()), 400, "terms nest at most 100 deep").await;
        // So is a model's calculated column, and its relationships' conditions.
        let mut deep_model = model();
        deep_model["models"][0]["columns"][6]["expression"] = json!(vec!["total"; 101].join(" + "));
        refused(&e, Query { manifest: deep_model, ..query("select sum(total) from orders", everything()) }, 400, "terms nest at most 100 deep").await;
        let mut deep_join = model();
        deep_join["relationships"][0]["condition"] = json!(format!("orders.region = regions.region{}", " and 1 = 1".repeat(101)));
        refused(&e, Query { manifest: deep_join, ..query("select sum(total) from orders", everything()) }, 400, "terms nest at most 100 deep").await;
        // Brackets past the parser's depth are refused too.
        refused(&e, query(&format!("select sum({}total{}) from orders", "(".repeat(300), ")".repeat(300)), everything()), 400, "does not parse").await;
        // As many models read as a metric could need, and no more.
        let reading =
            |n: usize| format!("select count(*) from orders o0{}", (1..n).map(|i| format!(" join orders o{i} on o0.id = o{i}.id")).collect::<String>());
        assert_eq!(one(&e, &reading(32), everything()).await.as_deref(), Some("4"));
        refused(&e, query(&reading(33), everything()), 400, "reads at most 32 models").await;
        let unions = format!("select count(*) from ({})", vec!["select 1 as x from orders"; 33].join(" union all "));
        refused(&e, query(&unions, everything()), 400, "reads at most 32 models").await;
        // The longest chain of relationships a semantic model can hold, each model's column read through the
        // next: answered, every order once. One model more is refused.
        let chain = |n: usize| {
            let models: Vec<Value> = (0..n)
                .map(|i| {
                    let mut columns = vec![json!({"name": "id", "type": "integer"}), json!({"name": "total", "type": "double"})];
                    if i + 1 < n {
                        columns.push(json!({"name": "nxt", "type": format!("m{}", i + 1), "relationship": format!("r{i}")}));
                        columns.push(json!({"name": "deep", "type": "double", "isCalculated": true, "expression": "nxt.deep"}));
                    } else {
                        columns.push(json!({"name": "deep", "type": "double", "isCalculated": true, "expression": "total"}));
                    }
                    json!({"name": format!("m{i}"), "tableReference": {"table": "orders_lake"}, "primaryKey": "id", "columns": columns})
                })
                .collect();
            let relationships: Vec<Value> = (0..n.saturating_sub(1))
                .map(|i| json!({"name": format!("r{i}"), "models": [format!("m{i}"), format!("m{}", i + 1)], "joinType": "ONE_TO_ONE", "condition": format!("m{i}.id = m{}.id", i + 1)}))
                .collect();
            json!({"catalog": "wren", "schema": "public", "models": models, "relationships": relationships})
        };
        let longest = Query { manifest: chain(32), ..query("select sum(deep) from m0", everything()) };
        let started = std::time::Instant::now();
        let answer = e.query(&longest).await.unwrap_or_else(|r| panic!("the longest chain: {r}"));
        assert_eq!(answer.rows, vec![vec![Some("185.5".to_string())]], "{answer:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "planned and run in {:?}", started.elapsed());
        refused(&e, Query { manifest: chain(33), ..query("select sum(deep) from m0", everything()) }, 400, "1 to 32 models").await;
    });
}
