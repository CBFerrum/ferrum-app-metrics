//! Metrics' service `metrics-engine`, the engine's token file (src/keys.rs; Ferrum docs/app-contract.md, "Declaring a
//! service"), against a real PostgreSQL (`FERRUM_METRICS_TEST_DATABASE_URL`; reset first): the token made by the first
//! render, once though two replicas render at once, bound to the engine's host; the engine started on the file written
//! (its tokens read as its `main` reads `FERRUM_METRICS_ENGINE_TOKEN_FILE`) takes what Metrics sends; a rotation's overlap
//! lets in the token before and the new one, and its end leaves the one before out; nothing set up, a closed vault: plain
//! refusals, nothing made or written; no token in a log, an error or the vault's record.

use ferrum_kernel::companion::{self, Rendered};
use ferrum_kernel::vault::{self, Kdf, New, Opener, Rule, Scope, Vault, Zeroizing};
use ferrum_kernel::{Db, DbConfig, Error, HookRegistry, Kernel, KernelSettings};
use ferrum_meta::Catalog;
use ferrum_metrics_engine::{Config, Engine, Limits};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::{Arc, Mutex};

fn url() -> String {
    std::env::var("FERRUM_METRICS_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_metrics_test".into())
}

/// A replica: its own pool, its vault opened with `database_url`'s password (none: nothing to make the first slot with),
/// Metrics registered as the server registers it; Metrics' handle, and the platform's (what the operator acts through).
async fn replica(database_url: String) -> (Arc<Kernel>, Vault, Vault) {
    let db = Db::connect(DbConfig { url: url(), max_connections: 8, ..Default::default() }).await.expect("connect");
    let opener = Opener { database_url, file: "/nonexistent/root.key".into(), kdf: Kdf { m_kib: 64, t: 1, p: 1 }, ..Default::default() };
    vault::open(&db, opener, false).await.expect("the vault");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_metrics::packages().expect("app packages"));
    let mut hooks = HookRegistry::default();
    let platform = hooks.platform_vault();
    let mut metrics = None;
    hooks.app("metrics", &[], |h| {
        ferrum_app_metrics::register(h);
        metrics = Some(h.vault());
    });
    let k = Kernel::boot(db, Catalog::from_packages(packages).expect("catalog"), hooks, KernelSettings::default(), None).await.expect("boot");
    (k, metrics.expect("Metrics' handle"), platform)
}

fn given(pairs: &[(&str, String)]) -> impl Fn(&str) -> Option<String> + Sync + use<> {
    let pairs: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    move |name: &str| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
}

fn token_file(r: &Rendered) -> String {
    assert_eq!(r.files.iter().map(|f| (f.name.as_str(), f.secret)).collect::<Vec<_>>(), [("token", true)]);
    String::from_utf8(r.files[0].bytes.to_vec()).unwrap()
}

async fn events(k: &Kernel, action: &str) -> Vec<Value> {
    vault::events(&k.db, None, 1000).await.unwrap().into_iter().filter(|e| e.action == action && e.name == "engine.token").map(|e| e.details).collect()
}

/// The engine, started on the file written for it: its tokens read as its `main` reads them. Stopped when dropped.
struct Running(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take() {
            let _ = stop.send(());
        }
    }
}

async fn engine_on(file: &Path, port: u16) -> Running {
    let tokens = ferrum_metrics_engine::tokens(file).expect("the engine's tokens");
    let engine = Arc::new(Engine::new(Config { datasets: vec![], limits: Limits::default() }).unwrap());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("the engine's port");
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(ferrum_metrics_engine::serve(listener, engine, tokens, async {
        let _ = stopped.await;
    }));
    Running(Some(stop))
}

/// Whether the engine took what Metrics sent: a check of nothing is refused for what it asks (invalid), not for its token.
async fn taken(k: &Kernel) -> bool {
    match ferrum_app_metrics::ask_engine(k, "check", &json!({})).await {
        Err(Error::Invalid(_) | Error::Validation(_)) => true,
        Err(e) if e.to_string().contains("refused its token") => false,
        other => panic!("{other:?}"),
    }
}

/// The engine's answer to a check sent with `token` by hand, as a browser would send it.
async fn status(port: u16, token: &str) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut sock = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.expect("the engine");
    let ask = format!(
        "POST /check HTTP/1.1\r\nhost: 127.0.0.1\r\nauthorization: Bearer {token}\r\naccept-encoding: gzip\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}"
    );
    sock.write_all(ask.as_bytes()).await.unwrap();
    let mut answer = Vec::new();
    sock.read_to_end(&mut answer).await.unwrap();
    String::from_utf8_lossy(&answer).split(' ').nth(1).unwrap().parse().unwrap()
}

/// Every log line of the test, as fields.
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<String>>);

struct Fields<'a>(&'a Mutex<String>);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0.lock().unwrap(), "{}={value:?} ", field.name());
    }
}

impl tracing::Subscriber for Logs {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        span.record(&mut Fields(&self.0));
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        values.record(&mut Fields(&self.0));
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        event.record(&mut Fields(&self.0));
        self.0.lock().unwrap().push('\n');
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn the_engines_token_file_is_rendered_from_the_vault() {
    let logs = Logs::default();
    tracing::subscriber::set_global_default(logs.clone()).expect("the test's logs");
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario(logs)).await.unwrap() });
}

async fn scenario(logs: Logs) {
    let db = Db::connect(DbConfig { url: url(), max_connections: 2, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists vault cascade; drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    // The second registered last: its handle is the one Metrics' own code keeps (keys.rs).
    let (k2, _, _) = replica(url()).await;
    let (k, metrics, platform) = replica(url()).await;
    let mut errors: Vec<String> = Vec::new();
    let listed = k.companions(&given(&[])).await;
    assert_eq!(
        listed.iter().map(|d| (d.name.as_str(), d.owner.as_str(), d.files.clone(), d.made.clone(), d.refused.is_some())).collect::<Vec<_>>(),
        [("metrics-engine", "metrics", vec!["token".to_string()], vec!["engine.token".to_string()], false)]
    );

    // ── nothing set up: said plainly, nothing made ──
    let e = k.render("metrics-engine", &given(&[])).await.unwrap_err().to_string();
    assert!(e.contains("the service metrics-engine needs the setting metrics.engine_url, which is not set"), "{e}");
    assert!(metrics.describe(&k, Scope::Installation, "engine.token").await.unwrap().is_none(), "nothing made");
    errors.push(e);
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let address = format!("http://127.0.0.1:{port}");
    ferrum_kernel::env::set([("FERRUM_METRICS_ENGINE_URL".to_string(), Some(address.clone()))].into());
    let e = ferrum_app_metrics::ask_engine(&k, "check", &json!({})).await.unwrap_err().to_string();
    assert!(e.contains("Metrics' vault holds no engine.token"), "{e}");
    errors.push(e);

    // ── made by the first render, once, though two replicas render at once; one line, bound to the engine's host ──
    let at = given(&[("metrics.engine_url", address.clone())]);
    let (a, b) = tokio::join!(k.render("metrics-engine", &at), k2.render("metrics-engine", &at));
    let (a, b) = (token_file(&a.expect("one replica")), token_file(&b.expect("the other")));
    assert!(a == b, "the same token in both");
    assert_eq!(events(&k, "make").await.len(), 1, "made once");
    let first = a.strip_suffix('\n').expect("a line").to_string();
    assert!(first.len() == 43 && !first.contains('\n'), "one token, 32 random bytes as base64url");
    let d = metrics.describe(&k, Scope::Installation, "engine.token").await.unwrap().unwrap();
    assert_eq!(
        (d.managed_by.as_deref(), d.rule.reveal, d.rule.overlap_secs, d.rule.hosts.clone()),
        (Some("metrics"), true, 86_400, vec!["127.0.0.1".to_string()])
    );
    let dir = std::env::temp_dir().join(format!("metrics-services-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let write = |r: Rendered| companion::write(&dir, &r).expect("write");
    write(k.render("metrics-engine", &at).await.unwrap());
    let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(dir.join("token")).unwrap().permissions()) & 0o777;
    assert_eq!(mode, 0o600);
    let running = engine_on(&dir.join("token"), port).await;
    assert!(taken(&k).await, "the engine started on the file takes what Metrics sends");

    // ── a rotation: the token before is still sent while the overlap lasts, and the engine restarted takes both ──
    let rotated = k.rotate_companion("metrics-engine", &at).await.expect("rotate");
    assert_eq!((rotated[0].version, rotated[0].previous_until.is_some()), (2, true));
    assert!(taken(&k).await, "the engine not restarted yet");
    let both = k.render("metrics-engine", &at).await.unwrap();
    let lines: Vec<String> = token_file(&both).lines().map(str::to_string).collect();
    assert!(lines.len() == 2 && lines[1] == first && lines[0] != first, "the new token first, then the one before");
    drop(running);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    write(both);
    let running = engine_on(&dir.join("token"), port).await;
    assert!(taken(&k).await && status(port, &lines[0]).await == 400 && status(port, &first).await == 400, "both let in");
    // The overlap over (as an hour's upkeep finds it): the new token is sent, and the one before is left out.
    let mut tx = k.db.pool.begin().await.unwrap();
    sqlx::query("select set_config('ferrum.vault', 'on', true)").execute(&mut *tx).await.unwrap();
    sqlx::query(
        "update vault.secret set valid_until = now() - interval '1 second' where owner = 'metrics' and name = 'engine.token' and valid_until is not null",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let last = k.render("metrics-engine", &at).await.unwrap();
    assert!(token_file(&last) == format!("{}\n", lines[0]), "the new token alone");
    drop(running);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    write(last);
    let running = engine_on(&dir.join("token"), port).await;
    assert!(taken(&k).await && status(port, &first).await == 401, "the token before is left out");
    drop(running);

    // ── kept, where the operator set the engine's token before a render made one ──
    metrics.remove(&k, Scope::Installation, "engine.token").await.unwrap();
    let typed = "the-engines-token-typed-for-both-0123456789";
    let at_engine = Rule { hosts: vec!["127.0.0.1".to_string()], ..Default::default() };
    platform.operator_set(&k, "metrics", "engine.token", New::Value(Zeroizing::new(typed.as_bytes().to_vec())), at_engine).await.expect("the operator's");
    let kept = k.render("metrics-engine", &given(&[])).await.expect("kept: nothing is made, so the address is not needed");
    assert!(token_file(&kept) == format!("{typed}\n"), "the token the engine already takes");
    assert_eq!(events(&k, "manage").await.len(), 1);

    // ── no token in a log, an error or the vault's record ──
    tracing::info!(probe = "metrics-services-it", "the test's own line");
    let logged = logs.0.lock().unwrap().clone();
    assert!(logged.contains("metrics-services-it"), "the logs are taken");
    let record = serde_json::to_string(&vault::events(&k.db, None, 1000).await.unwrap()).unwrap();
    for (i, token) in [first.as_str(), lines[0].as_str(), typed].iter().enumerate() {
        assert!(!logged.contains(token) && !record.contains(token) && !errors.iter().any(|e| e.contains(token)), "token {i} is out");
    }

    // ── a closed vault (a fresh database, and nothing to make its first slot with): refused, nothing written ──
    sqlx::raw_sql("drop schema if exists vault cascade; drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let (closed, _, _) = replica(url().replacen("ferrum:ferrum@", "ferrum@", 1)).await;
    let e = closed.render("metrics-engine", &at).await.unwrap_err().to_string();
    assert!(e.starts_with("the vault is closed"), "{e}");
    assert!(closed.rotate_companion("metrics-engine", &at).await.unwrap_err().to_string().starts_with("the vault is closed"));
    let _ = std::fs::remove_dir_all(&dir);
    ferrum_kernel::env::set(Default::default());
}
