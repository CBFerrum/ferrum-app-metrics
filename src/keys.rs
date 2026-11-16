//! The engine's token, in Metrics' own vault (Ferrum docs/app-contract.md, *The vault*): the installation's secret
//! `engine.token`, put on each request to the engine by the vault (`carry_at`), and only for the hosts its rule names.
//! The environment's `FERRUM_METRICS_ENGINE_TOKEN`, while it is set, still wins (through this release), and is adopted
//! into the vault once, bound to the host of the engine's address then: only where the vault holds none, under the
//! secret's own lock, so that replicas starting together, or a render making it at that moment, write it once.
//!
//! The service `metrics-engine` ([`Service`]) writes the token into the file the engine reads as it starts
//! (`FERRUM_METRICS_ENGINE_TOKEN_FILE`): made by the vault at the first render where it holds none, bound to the host of
//! `metrics.engine_url`, or the one there kept, from then on the service's, rotated by the operator through it.

use ferrum_kernel::companion::{Companion, File, Rendering};
use ferrum_kernel::vault::{New, Rule, Scope, Vault, Zeroizing};
use ferrum_kernel::{Error, Kernel, Result};
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// The engine's token, in the installation's scope of Metrics' vault.
pub const TOKEN: &str = "engine.token";

/// Metrics' handle on the vault, from its registration.
static VAULT: RwLock<Option<Vault>> = RwLock::new(None);
/// Whether this process has seen the environment's token in the vault already.
static ADOPTED: AtomicBool = AtomicBool::new(false);

/// Keeps the handle the registration gave Metrics.
// ponytail: the last registration's handle serves the whole process, as Drive's does.
pub(crate) fn remember(v: Vault) {
    *VAULT.write().unwrap_or_else(|e| e.into_inner()) = Some(v);
}

fn handle() -> Result<Vault> {
    VAULT.read().unwrap_or_else(|e| e.into_inner()).clone().ok_or_else(|| Error::Unavailable("Metrics was registered without its vault handle".into()))
}

/// The environment's token, while it sets one.
fn from_environment() -> Option<Zeroizing<String>> {
    ferrum_kernel::env::var("FERRUM_METRICS_ENGINE_TOKEN").filter(|t| !t.trim().is_empty()).map(Zeroizing::new)
}

/// The environment's token while it sets one (adopted into the vault once; a failure to keep it there is logged, and the
/// environment's is used all the same). `None`: the vault's is the one, put on the request by [`Vault::carry`].
pub(crate) async fn environment(k: &Kernel) -> Option<Zeroizing<String>> {
    let token = from_environment()?;
    if let Err(e) = adopt(k).await {
        tracing::warn!(error = %e, "Metrics could not keep the engine's token from the environment in its vault; it uses the environment's");
    }
    Some(token)
}

/// Metrics' handle, to carry the token with.
pub(crate) fn vault() -> Result<Vault> {
    handle()
}

/// Why the vault did not put the token on a request to `host`, in plain words.
pub(crate) fn refused(e: Error, host: &str) -> Error {
    match e {
        Error::NotFound(_) => Error::Unavailable(
            "Metrics' engine for external sources is not set up on this server: its token is in neither FERRUM_METRICS_ENGINE_TOKEN nor Metrics' vault (engine.token)"
                .into(),
        ),
        Error::Forbidden(_) => Error::Unavailable(format!(
            "Metrics' engine token may not be sent to {host}: the engine's address moved, so change the hosts of Metrics' secret engine.token to match"
        )),
        Error::Unavailable(why) | Error::Retryable(why) => {
            tracing::debug!("Metrics' vault: {why}");
            Error::Unavailable("Metrics cannot reach its engine's token right now: the vault is unavailable".into())
        }
        e => e,
    }
}

/// Takes the environment's token into the vault when the vault has none yet, bound to the host of the engine's address
/// now: once, only where there is none, under the secret's own lock. A token already there stays (the environment's,
/// while set, is the one used). Whether it set it now.
pub(crate) async fn adopt(k: &Kernel) -> Result<bool> {
    if ADOPTED.load(Ordering::Relaxed) {
        return Ok(false);
    }
    let (Some(token), Some(url)) = (from_environment(), crate::engine::address()) else { return Ok(false) };
    let Some(host) = host(&url) else { return Ok(false) };
    let rule = Rule { hosts: vec![host], ..Default::default() };
    let adopted = handle()?.adopt(k, Scope::Installation, TOKEN, New::Value(Zeroizing::new(token.as_bytes().to_vec())), rule, "environment").await?;
    ADOPTED.store(true, Ordering::Relaxed);
    Ok(adopted)
}

/// The version of the token to send: the one before while a rotation of the service's token overlaps, which an engine not
/// restarted since takes too; none (the current one) otherwise.
pub(crate) async fn sending(k: &Kernel) -> Result<Option<i32>> {
    let d = handle()?.describe(k, Scope::Installation, TOKEN).await?;
    Ok(d.filter(|d| d.managed_by.is_some() && d.previous_until.is_some_and(|t| t > chrono::Utc::now())).map(|d| d.version - 1))
}

/// How long the token before a rotation is still sent and still taken: a day, to restart the engine's pods in.
const OVERLAP_SECS: i64 = 86_400;

/// The engine's token file, written as each engine starts (`ferrum task render service:metrics-engine to:<dir>`): `token`,
/// each version the vault keeps on a line of its own, the current first. Made by the vault at the first render where it
/// holds none, bound to the host of `metrics.engine_url`; where it holds one that is no service's yet (Metrics 0.8 copied
/// it from `FERRUM_METRICS_ENGINE_TOKEN`, or a person set it: the token the engine already takes), kept as it is, with the
/// hosts it had. One a render made or kept before is left as it is.
pub(crate) struct Service;

#[async_trait::async_trait]
impl Companion for Service {
    fn files(&self) -> &'static [&'static str] {
        &["token"]
    }

    fn made(&self) -> &'static [&'static str] {
        &[TOKEN]
    }

    async fn render(&self, r: &Rendering<'_>) -> Result<Vec<File>> {
        let rule = Rule { reveal: true, overlap_secs: OVERLAP_SECS, ..Default::default() };
        match r.vault().describe(r.kernel(), Scope::Installation, TOKEN).await? {
            None => {
                let url = r.needs("metrics.engine_url")?;
                let host = host(&url).ok_or_else(|| Error::invalid("metrics.engine_url is the engine's address, such as http://metrics-engine:5180"))?;
                r.made(TOKEN, Rule { hosts: vec![host], ..rule }).await?
            }
            Some(d) if d.managed_by.is_none() => r.keep(TOKEN, Rule { hosts: d.rule.hosts, ..rule }).await?,
            Some(_) => {}
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(256));
        for v in r.versions(TOKEN).await? {
            bytes.extend_from_slice(&v.value);
            bytes.push(b'\n');
        }
        Ok(vec![File { name: "token".into(), bytes, secret: true }])
    }
}

/// The host of an address (`http://engine:5180/x` → `engine`), in lower case.
fn host(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://")?.1;
    let at = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
    let h = match at.strip_prefix('[') {
        Some(v6) => v6.split(']').next()?,
        None => at.split(':').next()?,
    };
    (!h.is_empty()).then(|| h.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_address_names_its_host() {
        assert_eq!(super::host("http://User@Engine.Local:5180/check").as_deref(), Some("engine.local"));
        assert_eq!(super::host("http://[::1]:5180").as_deref(), Some("::1"));
        assert_eq!(super::host("engine:5180"), None);
    }
}
