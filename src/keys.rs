//! The engine's token, in Metrics' own vault (Ferrum docs/app-contract.md, *The vault*): the installation's secret
//! `engine.token`, never revealed, put on each request to the engine by the vault (`carry`), and only for the hosts its
//! rule names. The environment's `FERRUM_METRICS_ENGINE_TOKEN`, while it is set, still wins (through this release), and
//! is adopted into the vault once, bound to the host of the engine's address then: under an advisory lock, so that
//! replicas starting together adopt it once. Moving the engine's address to another host does not move the token with
//! it: the operator changes the secret's rule (`PUT /api/operator/secrets/metrics/engine.token/rule`).

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
/// now: once, under an advisory lock held while it looks and sets, so that replicas doing it together set it once. A
/// token already there stays (the environment's, while set, is the one used). Whether it set it now.
pub(crate) async fn adopt(k: &Kernel) -> Result<bool> {
    if ADOPTED.load(Ordering::Relaxed) {
        return Ok(false);
    }
    let (Some(token), Some(url)) = (from_environment(), crate::engine::address()) else { return Ok(false) };
    let Some(host) = host(&url) else { return Ok(false) };
    let v = handle()?;
    let mut lock = k.db.system_tx().await?;
    sqlx::query("select pg_advisory_xact_lock(hashtext('metrics: adopt engine.token'))").execute(&mut *lock).await?;
    let adopted = match v.describe(k, Scope::Installation, TOKEN).await? {
        Some(_) => false,
        None => {
            let rule = Rule { hosts: vec![host], ..Default::default() };
            v.set(k, Scope::Installation, TOKEN, New::Value(Zeroizing::new(token.as_bytes().to_vec())), rule).await?;
            true
        }
    };
    lock.commit().await?;
    ADOPTED.store(true, Ordering::Relaxed);
    Ok(adopted)
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
