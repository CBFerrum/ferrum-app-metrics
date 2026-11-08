//! Metrics' engine as a service: its token, its limits and its answers over HTTP, as Metrics calls it.

use ferrum_metrics_engine::{Config, Dataset, Engine, Format, Limits, serve};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::sync::Arc;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

/// One request over a plain connection: its status and body.
fn http(addr: SocketAddr, method: &str, path: &str, token: Option<&str>, body: &[u8]) -> (u16, String) {
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: engine\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    // A body refused for its size may be cut off while it is still being sent: the answer is what counts.
    let _ = s.write_all(head.as_bytes()).and_then(|_| s.write_all(body));
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let text = String::from_utf8_lossy(&out).to_string();
    let status = text.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_engine_over_http() {
    let dir = std::env::temp_dir().join(format!("ferrum-engine-http-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("orders.csv"), "id,total\n1,100.00\n2,50.00\n").unwrap();
    let config = Config {
        datasets: vec![Dataset { name: "orders_lake".into(), workspace: "acme".into(), format: Format::Csv, path: dir.join("orders.csv") }],
        limits: Limits::default(),
    };
    let engine = Arc::new(Engine::new(config).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(serve(listener, engine, TOKEN.into(), async {
        let _ = stopped.await;
    }));
    let manifest = json!({"catalog": "wren", "schema": "public", "models": [{"name": "orders", "tableReference": {"table": "orders_lake"}, "columns": [{"name": "id", "type": "integer"}, {"name": "total", "type": "double"}]}]});
    let ask = |sql: &str| serde_json::to_vec(&json!({"workspace": "acme", "manifest": manifest, "sql": sql, "datasets": [{"name": "orders_lake"}]})).unwrap();

    let (status, body) = tokio::task::spawn_blocking(move || http(addr, "GET", "/health", None, b"")).await.unwrap();
    assert_eq!(status, 200);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["datasets"], 1);
    let call = |path: &'static str, token: Option<&'static str>, body: Vec<u8>| tokio::task::spawn_blocking(move || http(addr, "POST", path, token, &body));
    // Metrics' token, whole, or nothing.
    assert_eq!(call("/query", None, ask("select sum(total) from orders")).await.unwrap().0, 401);
    assert_eq!(call("/query", Some("0123456789abcdef0123456789abcdeX"), ask("select sum(total) from orders")).await.unwrap().0, 401);
    let (status, body) = call("/query", Some(TOKEN), ask("select sum(total) from orders")).await.unwrap();
    assert_eq!(status, 200, "{body}");
    let answer: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["rows"], json!([["150.0"]]), "{answer}");
    assert!(answer["datasets"][0]["written_at"].is_string(), "{answer}");
    // Refusals carry their reason and their status.
    let (status, body) = call("/query", Some(TOKEN), ask("drop table orders")).await.unwrap();
    assert_eq!(status, 400);
    assert!(serde_json::from_str::<Value>(&body).unwrap()["error"].as_str().unwrap().contains("only a single SELECT"), "{body}");
    let (status, body) = call("/query", Some(TOKEN), b"{\"workspace\": \"acme\"}".to_vec()).await.unwrap();
    assert_eq!(status, 400, "{body}");
    let (status, body) = call("/check", Some(TOKEN), ask("select sum(total) from orders")).await.unwrap();
    assert_eq!(status, 200, "{body}");
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["columns"][0]["numeric"], true, "{body}");
    // Too large is refused before it is read.
    let huge = vec![b' '; ferrum_metrics_engine::MOST_BODY + 1];
    assert_eq!(call("/query", Some(TOKEN), huge).await.unwrap().0, 413);
    // Asked to stop, it stops.
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
}
