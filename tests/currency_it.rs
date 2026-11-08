//! Currencies, fiscal years and fan-out against hand-worked numbers (Ferrum's Logic Platform: FLC-SEM-004,
//! FLC-QUAL-005). Every expected value below was worked out by hand from the rates and sales written
//! here, never by the code it checks, so a wrong rate rule, rounding, refund or fiscal boundary in the
//! implementation shows up as a different number. Against a real PostgreSQL
//! (`FERRUM_METRICS_TEST_DATABASE_URL`).
//!
//! The rates: USD→INR 83.10 from 1 March 2026 and 84.25 from 1 April 2026; INR→GBP 0.0095 from 1 January
//! 2026 and nothing from GBP, so GBP→INR is its inverse, 1 / 0.0095 = 105.263157895 to 9 places.
//!
//! | sale | who | amount | day | rate | INR (exact) |
//! |---|---|---|---|---|---|
//! | s1 | clerk A | 100.00 USD (lines: Pen 60.00, Pen 40.00) | 2026-03-31 | 83.10 | 8310.00 |
//! | s2 | clerk A | 50.00 USD (line: Ink 50.00) | 2026-04-02 | 84.25 | 4212.50 |
//! | s3 | clerk B | 1000.00 INR | 2026-04-15 | 1 | 1000.00 |
//! | s5 | clerk B | 10.00 GBP | 2026-03-20 | 105.263157895 | 1052.63157895 |
//! | s6 | clerk A | 0.10 USD | 2026-04-01, the day the new rate starts | 84.25 | 8.425 |
//! | s4 | clerk A | -20.00 USD, a refund entered last but dated back | 2026-03-15 | 83.10 | -1662.00 |
//!
//! All of them: 12921.55657895 → 12921.56. Clerk A: 10868.925 → 10868.92 (a tie, to the even digit;
//! half up would say .93). Clerk B: 2052.63157895 → 2052.63. Fiscal years from April: 2025-26 holds
//! s1, s4, s5 = 7700.63157895 → 7700.63; 2026-27 holds s2, s3, s6 = 5220.925 → 5220.92 (a tie again).
//! Clerk A's: 6648 and 4220.925 → 4220.92.

use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings, ListQuery};
use ferrum_meta::Catalog;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

/// A small shop of this test's own: sales in several currencies, with lines.
const SHOP: &[(&str, &str, &str)] = &[
    (
        "shop",
        "package.yaml",
        r#"package: shop
version: 0.1.0
label: Shop
depends: [core]
modules:
  - name: shop
    label: Shop
    icon: store
    order: 90
    items: [Sale]
roles:
  - name: Shop Clerk
    description: Sells, and sees their own sales.
    permissions:
      - {entity: Sale, read: true, create: true, write: true, if_owner: true}
  - name: Shop Manager
    description: Sees every sale.
    permissions:
      - {entity: Sale, read: true, create: true, write: true}
"#,
    ),
    (
        "shop",
        "entities/sale.yaml",
        r#"- name: Sale
  label: Sale
  kind: master
  module: shop
  fields:
    - {name: amount, type: currency, currency: currency, required: true}
    - {name: currency, type: link, to: Currency, required: true}
    - {name: posted_on, type: date, required: true}
    - {name: lines, type: table, of: SaleLine}
- name: SaleLine
  label: Sale Line
  kind: child
  fields:
    - {name: item, type: data, required: true}
    - {name: amount, type: currency, required: true}
    - {name: currency, type: link, to: Currency}
"#,
    ),
];

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

/// A metric's value as its reader gets it, exactly as written.
async fn value(k: &Kernel, who: &Ctx, metric: &str) -> Result<String, Error> {
    let v = ask(k, who, "/value", json!({"metric": metric})).await?;
    Ok(v["value"].as_str().expect("a value as exact text").to_string())
}

/// A series' points by their first day.
async fn series(k: &Kernel, who: &Ctx, metric: &str, bucket: &str, periods: u32) -> Result<std::collections::BTreeMap<String, Value>, Error> {
    let s = ask(k, who, "/series", json!({"metric": metric, "bucket": bucket, "periods": periods.to_string()})).await?;
    Ok(s["points"].as_array().unwrap().iter().map(|p| (p["start"].as_str().unwrap().to_string(), p["value"].clone())).collect())
}

fn refused<V: std::fmt::Debug>(r: Result<V, Error>, says: &str) {
    let msg = format!("{r:?}");
    assert!(r.is_err() && msg.contains(says), "expected a refusal saying {says:?}, got {msg}");
}

#[test]
fn currencies_fiscal_years_and_fan_out_against_hand_worked_numbers() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_METRICS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_metrics_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 16, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_metrics::packages().expect("app packages"));
    packages.extend(ferrum_meta::catalog::load_embedded(SHOP).expect("the shop"));
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_metrics::register(&mut hooks);
    let settings = KernelSettings { secret_key: Some(SecretKey::new([7u8; 32])), ..Default::default() };
    let k: Arc<Kernel> = Kernel::boot(db, Catalog::from_packages(packages).expect("catalog"), hooks, settings, None).await.expect("boot");
    ferrum_domain::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "rupees".into(),
            name: "Rupees Ltd".into(),
            admin_email: "admin@rupees.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["metrics".into(), "shop".into()],
            settings: Some(json!({"timezone": "UTC", "fiscal_year_start_month": 4})),
        })
        .await
        .expect("tenant");
    let session = k.login("rupees", "admin@rupees.test", "Correct-Horse-9").await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let user = |email: &'static str, roles: &'static [&'static str]| {
        let (k, admin) = (k.clone(), admin.clone());
        async move {
            let roles: Vec<String> = roles.iter().map(|r| r.to_string()).collect();
            let id = k.create_user(&admin, email, email, Some("Correct-Horse-9"), &roles, false).await.expect("user");
            k.load_ctx(tenant, id, None, "it".into()).await.expect("ctx")
        }
    };
    let author = user("analyst@rupees.test", &["Metric Author", "Shop Manager"]).await;
    let steward = user("steward@rupees.test", &["Data Steward", "Shop Manager"]).await;
    let manager = user("manager@rupees.test", &["Metric Reader", "Shop Manager"]).await;
    let clerk_a = user("a@rupees.test", &["Metric Reader", "Shop Clerk"]).await;
    let clerk_b = user("b@rupees.test", &["Metric Reader", "Shop Clerk"]).await;
    let clerk_c = user("c@rupees.test", &["Metric Reader", "Shop Clerk"]).await;

    // ───────────── currencies, rates and sales ─────────────
    // A workspace may come with some currencies: each taken by its code, else added.
    let currency = |code: &'static str, name: &'static str| {
        let (k, admin) = (k.clone(), admin.clone());
        async move {
            let mut tx = k.db.tenant_tx(admin.tenant).await.unwrap();
            let found = k.find_tx(&admin, &mut tx, "Currency", &[ferrum_kernel::Filter::eq("code", code)], None, 1).await.unwrap();
            tx.commit().await.unwrap();
            match found.first() {
                Some(c) => uid(&c["id"]),
                None => uid(&k.insert(&admin, "Currency", obj(json!({"code": code, "currency_name": name, "precision": 2}))).await.expect("currency")["id"]),
            }
        }
    };
    let (inr, usd, gbp, eur) =
        (currency("INR", "Indian Rupee").await, currency("USD", "US Dollar").await, currency("GBP", "Pound").await, currency("EUR", "Euro").await);
    let rate = |from: Uuid, to: Uuid, rate: &str, valid_from: &str| {
        let (k, admin) = (k.clone(), admin.clone());
        let doc = obj(json!({"from_currency": from.to_string(), "to_currency": to.to_string(), "rate": rate, "valid_from": valid_from}));
        async move { k.insert(&admin, "ExchangeRate", doc).await.expect("rate") }
    };
    rate(usd, inr, "83.10", "2026-03-01").await;
    rate(usd, inr, "84.25", "2026-04-01").await;
    rate(inr, gbp, "0.0095", "2026-01-01").await;
    let sale = |who: &Ctx, amount: &str, currency: Uuid, day: &str, lines: Value| {
        let (k, who) = (k.clone(), who.clone());
        let doc = obj(json!({"amount": amount, "currency": currency.to_string(), "posted_on": day, "lines": lines}));
        async move { k.insert(&who, "Sale", doc).await.expect("sale") }
    };
    let line = |item: &str, amount: &str| json!({"item": item, "amount": amount, "currency": usd.to_string()});
    sale(&clerk_a, "100.00", usd, "2026-03-31", json!([line("Pen", "60.00"), line("Pen", "40.00")])).await;
    sale(&clerk_a, "50.00", usd, "2026-04-02", json!([line("Ink", "50.00")])).await;
    sale(&clerk_b, "1000.00", inr, "2026-04-15", json!([])).await;
    sale(&clerk_b, "10.00", gbp, "2026-03-20", json!([])).await;
    sale(&clerk_a, "0.10", usd, "2026-04-01", json!([])).await;
    // A refund entered after April's sales, dated back into March: the year it belongs to.
    sale(&clerk_a, "-20.00", usd, "2026-03-15", json!([])).await;

    // ───────────── converted metrics, checked when saved ─────────────
    let make = |name: &str, body: Value| {
        let (k, author) = (k.clone(), author.clone());
        let mut doc = body.as_object().cloned().unwrap();
        doc.insert("metric_name".into(), json!(name));
        async move { k.insert(&author, "Metric", doc).await }
    };
    let certify = |m: &Doc| {
        let (k, steward, id) = (k.clone(), steward.clone(), uid(&m["id"]));
        async move { k.run_action(&steward, "Metric", id, "certify", Value::Null).await.expect("certified") }
    };
    let sum = |field: &str, filters: Value| json!({"entity": "Sale", "filters": filters, "measures": [{"fn": "sum", "field": field}]});
    let in_inr = |definition: Value| json!({"definition": definition, "convert_to": inr.to_string(), "currency_field": "currency", "rate_date_field": "posted_on", "series_by": "posted_on", "unit": "INR"});
    let sales = make("Sales in INR", in_inr(sum("amount", Value::Null))).await.expect("a converted metric");
    assert_eq!(sales["uses"], "Sale: amount; converted by currency at the rate on posted_on", "{sales:?}");
    assert_eq!(sales["round_to"], json!(2));

    refused(make("Counted", in_inr(json!({"entity": "Sale", "measures": [{"fn": "count"}]}))).await, "a converted metric sums an amount");
    let mut by_day = in_inr(sum("amount", Value::Null));
    by_day["currency_field"] = json!("posted_on");
    refused(make("By day", by_day).await, "posted_on does not link to a currency");
    refused(
        make("Half set", json!({"definition": sum("amount", Value::Null), "convert_to": inr.to_string()})).await,
        "say which field holds each row's currency",
    );
    refused(make("Other half", json!({"definition": sum("amount", Value::Null), "currency_field": "currency"})).await, "say which currency to convert to");
    let mut too_fine = in_inr(sum("amount", Value::Null));
    too_fine["round_to"] = json!(7);
    assert!(make("Too fine", too_fine).await.is_err(), "rounded to 0 to 6 places");
    let mut nowhere = in_inr(sum("amount", Value::Null));
    nowhere["convert_to"] = json!(Uuid::now_v7().to_string());
    assert!(make("Nowhere", nowhere).await.is_err(), "into a currency that is not here");
    // A currency on the lines would count each sale once per line: refused, never multiplied.
    let mut per_line = in_inr(sum("amount", Value::Null));
    per_line["currency_field"] = json!("lines.currency");
    refused(make("By line currency", per_line).await, "once per lines row");
    // A formula over money keeps to one currency; a composed metric converts by its parts.
    let usd_sales = make(
        "Sales in USD",
        json!({"definition": sum("amount", Value::Null), "convert_to": usd.to_string(), "currency_field": "currency", "rate_date_field": "posted_on"}),
    )
    .await
    .unwrap();
    refused(make("Mixed", json!({"formula": "[Sales in INR] + [Sales in USD]"})).await, "convert to different currencies");
    refused(
        make(
            "Composed and converted",
            json!({"formula": "[Sales in INR] * 2", "convert_to": inr.to_string(), "currency_field": "currency", "rate_date_field": "posted_on"}),
        )
        .await,
        "converted by its parts",
    );
    assert_ne!(sales["id"], usd_sales["id"]);

    let balance = make("Sales to 31 March 2026", in_inr(sum("amount", json!([["posted_on", "<=", "2026-03-31"]])))).await.unwrap();
    let pens = make("Pen sales in INR", in_inr(sum("amount", json!([["lines.item", "eq", "Pen"]])))).await.unwrap();
    let lines = make("Line sales in INR", in_inr(sum("lines.amount", Value::Null))).await.unwrap();
    for m in [&sales, &balance, &pens, &lines] {
        certify(m).await;
    }

    // ───────────── each reader's own sales, converted day by day ─────────────
    assert_eq!(value(&k, &manager, "Sales in INR").await.unwrap(), "12921.56");
    assert_eq!(value(&k, &clerk_a, "Sales in INR").await.unwrap(), "10868.92", "10868.925 to the even digit");
    assert_eq!(value(&k, &clerk_b, "Sales in INR").await.unwrap(), "2052.63", "a pound at the inverse of the rupee's rate");
    assert_eq!(value(&k, &clerk_c, "Sales in INR").await.unwrap(), "0", "nothing of their own");
    // The backdated refund is in March, so in the balance to 31 March.
    assert_eq!(value(&k, &manager, "Sales to 31 March 2026").await.unwrap(), "7700.63");
    assert_eq!(value(&k, &clerk_a, "Sales to 31 March 2026").await.unwrap(), "6648");
    // A sale with two Pen lines is one sale: counted once, never once per line.
    assert_eq!(value(&k, &manager, "Pen sales in INR").await.unwrap(), "8310");
    // The lines themselves, each at its sale's rate: (60 + 40) × 83.10 + 50 × 84.25.
    assert_eq!(value(&k, &manager, "Line sales in INR").await.unwrap(), "12522.5");

    // ───────────── fiscal years from April ─────────────
    let today = chrono::Utc::now().date_naive();
    let fy_now = if chrono::Datelike::month(&today) >= 4 { chrono::Datelike::year(&today) } else { chrono::Datelike::year(&today) - 1 };
    let years = u32::try_from(fy_now - 2025 + 1).unwrap();
    let fy = series(&k, &manager, "Sales in INR", "fiscal_year", years).await.unwrap();
    assert!(fy.len() == years as usize && fy.keys().all(|start| start.ends_with("-04-01")), "one point a fiscal year, each from April: {fy:?}");
    assert_eq!((fy.get("2025-04-01"), fy.get("2026-04-01")), (Some(&json!("7700.63")), Some(&json!("5220.92"))), "{fy:?}");
    let fy_a = series(&k, &clerk_a, "Sales in INR", "fiscal_year", years).await.unwrap();
    assert_eq!((fy_a.get("2025-04-01"), fy_a.get("2026-04-01")), (Some(&json!("6648")), Some(&json!("4220.92"))), "{fy_a:?}");
    // Fiscal quarters start in April, July, October and January.
    let quarters = u32::try_from((fy_now - 2025) * 4 + 4).unwrap();
    let fq = series(&k, &manager, "Sales in INR", "fiscal_quarter", quarters).await.unwrap();
    assert!(fq.len() == quarters as usize && fq.keys().all(|start| ["-01-01", "-04-01", "-07-01", "-10-01"].iter().any(|q| start.ends_with(q))), "{fq:?}");
    assert_eq!(
        (fq.get("2026-01-01"), fq.get("2026-04-01"), fq.get("2026-07-01")),
        (Some(&json!("7700.63")), Some(&json!("5220.92")), Some(&json!("0"))),
        "{fq:?}"
    );
    // And calendar months: March holds the refund, April the day the new rate starts.
    let months = u32::try_from((chrono::Datelike::year(&today) - 2026) * 12 + i32::try_from(chrono::Datelike::month(&today)).unwrap() - 2).unwrap();
    let by_month = series(&k, &clerk_a, "Sales in INR", "month", months).await.unwrap();
    assert!(by_month.len() == months as usize && by_month.keys().all(|start| start.ends_with("-01")), "{by_month:?}");
    assert_eq!((by_month.get("2026-03-01"), by_month.get("2026-04-01")), (Some(&json!("6648")), Some(&json!("4220.92"))), "{by_month:?}");

    // ───────────── a currency without a rate ─────────────
    sale(&clerk_c, "5.00", eur, "2026-04-20", json!([])).await;
    refused(value(&k, &clerk_c, "Sales in INR").await, "no exchange rate from EUR to INR on or before 2026-04-20");
    refused(value(&k, &manager, "Sales in INR").await, "no exchange rate from EUR to INR");
    refused(series(&k, &manager, "Sales in INR", "fiscal_year", years).await, "no exchange rate from EUR to INR");
    // Clerk A has no euros: their value stands.
    assert_eq!(value(&k, &clerk_a, "Sales in INR").await.unwrap(), "10868.92");
    rate(eur, inr, "90.00", "2026-04-20").await;
    assert_eq!(value(&k, &clerk_c, "Sales in INR").await.unwrap(), "450");
    assert_eq!(value(&k, &manager, "Sales in INR").await.unwrap(), "13371.56", "12921.55657895 + 450");

    // ───────────── what a kept value meant ─────────────
    let kept = k.run_action(&manager, "Metric", uid(&sales["id"]), "snapshot", Value::Null).await.expect("kept");
    assert_eq!(kept["value"], "13371.56");
    let was = k.get(&author, "Metric", uid(&sales["id"]), false).await.unwrap();
    // Rounding is part of what is certified: changing it is a draft to certify again.
    k.update(&author, "Metric", uid(&sales["id"]), obj(json!({"round_to": 0})), None).await.expect("rounded to whole rupees");
    let now = k.get(&author, "Metric", uid(&sales["id"]), false).await.unwrap();
    assert_eq!((now["status"].as_str(), now["certified_hash"].as_str()), (Some("Draft"), None), "{now:?}");
    assert_eq!(value(&k, &author, "Sales in INR").await.unwrap(), "13372", "its owner reads the draft: 13371.55657895 to no places");
    refused(value(&k, &manager, "Sales in INR").await, "NotFound");
    // The value kept before says what the metric meant then.
    let snaps = k.list(&manager, "MetricSnapshot", &ListQuery::default()).await.unwrap().rows;
    let snap = snaps.iter().find(|s| s["metric"] == sales["id"]).expect("the manager's kept value");
    assert_eq!((snap["value"].as_str(), snap["certified_hash"].as_str()), (Some("13371.56"), was["certified_hash"].as_str()));
    assert_eq!(
        (&snap["definition"]["round_to"], &snap["definition"]["convert_to"], &snap["definition"]["definition"]["measures"][0]["field"]),
        (&json!(2), &json!(inr.to_string()), &json!("amount")),
        "{snap:?}"
    );
    // A kept value is its reader's: no one else's role reads it.
    for who in [&steward, &author, &clerk_a] {
        let seen = k.list(who, "MetricSnapshot", &ListQuery::default()).await.unwrap().rows;
        assert!(seen.iter().all(|s| s["read_by"] == json!(who.email)), "{} sees only their own values: {seen:?}", who.email);
    }
    assert_eq!(k.list(&admin, "MetricSnapshot", &ListQuery::default()).await.unwrap().rows.len(), snaps.len());
}
