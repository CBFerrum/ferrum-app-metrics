//! `ferrum-metrics-engine`: Metrics' engine for external sources, as a service.
//!
//! * `FERRUM_METRICS_ENGINE_CONFIG`: the operator's JSON file of datasets and limits (see the crate's docs).
//! * `FERRUM_METRICS_ENGINE_TOKEN`: the bearer token Metrics sends, at least 32 characters.
//! * `FERRUM_METRICS_ENGINE_ADDR`: where it listens; `127.0.0.1:5180` unless set.
//!
//! It stops taking work on SIGTERM or Ctrl-C and finishes what it holds.

use ferrum_metrics_engine::{Config, Engine, STACK, serve};
use std::sync::Arc;

fn main() {
    // Queries run on these threads, each with the stack a query at the largest size taken needs.
    let done = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(STACK)
        .build()
        .map_err(|e| format!("cannot start: {e}"))
        .and_then(|runtime| runtime.block_on(run()));
    if let Err(e) = done {
        eprintln!("ferrum-metrics-engine: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let path = std::env::var("FERRUM_METRICS_ENGINE_CONFIG").map_err(|_| "FERRUM_METRICS_ENGINE_CONFIG names no configuration file".to_string())?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{path} cannot be read: {e}"))?;
    let config: Config = serde_json::from_str(&text).map_err(|e| format!("{path} is not a configuration: {e}"))?;
    let token = std::env::var("FERRUM_METRICS_ENGINE_TOKEN").map_err(|_| "FERRUM_METRICS_ENGINE_TOKEN is not set".to_string())?;
    if token.len() < 32 {
        return Err("FERRUM_METRICS_ENGINE_TOKEN is at least 32 characters".into());
    }
    let addr = std::env::var("FERRUM_METRICS_ENGINE_ADDR").unwrap_or_else(|_| "127.0.0.1:5180".into());
    for d in &config.datasets {
        if !d.path.exists() {
            eprintln!("ferrum-metrics-engine: dataset {} has nothing at {} yet; queries of it say so", d.name, d.path.display());
        }
    }
    let engine = Arc::new(Engine::new(config)?);
    let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| format!("cannot listen on {addr}: {e}"))?;
    eprintln!("ferrum-metrics-engine: listening on {addr} with {} datasets", engine.datasets());
    serve(listener, engine, token, stopped()).await.map_err(|e| e.to_string())
}

/// SIGTERM or Ctrl-C.
async fn stopped() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = interrupt => {},
        _ = terminate => {},
    }
    eprintln!("ferrum-metrics-engine: stopping");
}
