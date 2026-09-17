//! Local BTC Lightning protected resource, alongside the original simulated RGB demo.
use axum::{
    extract::{OriginalUri, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use macaroon::{Caveat, Format, Macaroon, MacaroonKey, Verifier};
use rgb402_payment::{
    commerce::{now, L402Challenge},
    lightning::{LightningInvoice, LightningNode},
    wallet::WalletError,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
    sync::Arc,
};
use tokio::sync::Mutex;

pub struct Merchant {
    node: Arc<dyn LightningNode>,
    key: MacaroonKey,
    cached: Mutex<HashMap<String, Challenge>>,
}
#[derive(Clone)]
struct Challenge {
    invoice: LightningInvoice,
    macaroon: String,
}
impl Merchant {
    pub fn open(node: Arc<dyn LightningNode>, key_path: &Path) -> Result<Self, WalletError> {
        macaroon::initialize()
            .map_err(|_| WalletError::Invalid("macaroon initialization failed"))?;
        let mut bytes = [0; 32];
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(key_path)
        {
            Ok(mut file) => {
                File::open("/dev/urandom")?.read_exact(&mut bytes)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                File::open(
                    key_path
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(Path::new(".")),
                )?
                .sync_all()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut file = File::open(key_path)?;
                if file.metadata()?.len() != 32 {
                    return Err(WalletError::Invalid("invalid merchant signing key file"));
                }
                file.read_exact(&mut bytes)?;
            }
            Err(e) => return Err(e.into()),
        }
        Ok(Self {
            node,
            key: MacaroonKey::generate(&bytes),
            cached: Mutex::new(HashMap::new()),
        })
    }
    async fn challenge(&self, path: &str, amount: u64) -> Result<Challenge, WalletError> {
        // Serialize issuance: simultaneous unpaid requests share one short-lived invoice.
        let mut cache = self.cached.lock().await;
        if let Some(c) = cache
            .get(path)
            .filter(|c| c.invoice.expires_at > now() + 30)
        {
            return Ok(c.clone());
        }
        let invoice = self.node.create_invoice(amount, 600).await?;
        let macaroon = self.mint(path, &invoice)?;
        let c = Challenge { invoice, macaroon };
        cache.insert(path.into(), c.clone());
        Ok(c)
    }
    fn mint(&self, path: &str, invoice: &LightningInvoice) -> Result<String, WalletError> {
        // L402 v0 identifier: version (u16), payment hash (32), random user ID (32).
        let hash = hex::decode(invoice.payment_hash.as_str())
            .map_err(|_| WalletError::Invalid("invalid invoice hash"))?;
        if hash.len() != 32 {
            return Err(WalletError::Invalid("invalid invoice hash"));
        }
        let mut identifier = vec![0, 0];
        identifier.extend(hash);
        let mut nonce = [0; 32];
        File::open("/dev/urandom")?.read_exact(&mut nonce)?;
        identifier.extend(nonce);
        let mut m = Macaroon::create(None, &self.key, identifier.into())
            .map_err(|_| WalletError::Invalid("macaroon creation failed"))?;
        m.add_first_party_caveat(format!("resource = {path}").into());
        // Paid access is reusable for one hour, including merchant restarts with the same key.
        m.add_first_party_caveat(format!("expires = {}", now() + 3600).into());
        m.serialize(Format::V2)
            .map_err(|_| WalletError::Invalid("macaroon serialization failed"))
    }
    fn authorized(&self, path: &str, auth: &str, at: u64) -> bool {
        if auth.len() > 16_384 {
            return false;
        }
        let Some((token, proof)) = auth.strip_prefix("L402 ").and_then(|s| s.split_once(':'))
        else {
            return false;
        };
        let Ok(proof) = hex::decode(proof) else {
            return false;
        };
        if proof.len() != 32 {
            return false;
        }
        let Ok(m) = Macaroon::deserialize(token) else {
            return false;
        };
        let id = m.identifier();
        if id.0.len() != 66 || id.0[..2] != [0, 0] || id.0[2..34] != Sha256::digest(proof)[..] {
            return false;
        }
        let mut verifier = Verifier::default();
        verifier.satisfy_exact(format!("resource = {path}").into());
        let mut resource = false;
        let mut expiry = false;
        for caveat in m.caveats() {
            let Caveat::FirstParty(c) = caveat else {
                return false;
            };
            let Ok(predicate) = String::from_utf8(c.predicate().0) else {
                return false;
            };
            if predicate == format!("resource = {path}") {
                resource = true;
            }
            if let Some(end) = predicate
                .strip_prefix("expires = ")
                .and_then(|s| s.parse::<u64>().ok())
            {
                if end <= at {
                    return false;
                }
                verifier.satisfy_exact(predicate.into());
                expiry = true;
            }
        }
        resource && expiry && verifier.verify(&m, &self.key, vec![]).is_ok()
    }
}
pub fn router(merchant: Merchant) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route(
            "/public",
            get(|| async {
                Json(serde_json::json!({"tier":"public","message":"Free demo resource"}))
            }),
        )
        .route("/premium/report", get(protected))
        .route("/premium/extended", get(protected))
        .with_state(Arc::new(merchant))
}
async fn protected(
    State(merchant): State<Arc<Merchant>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let path = uri.path();
    if uri.query().is_some() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| merchant.authorized(path, h, now()))
    {
        return ([(header::CACHE_CONTROL,"no-store")],Json(serde_json::json!({"pair":"BTC/USD","source":"RGB402 Demo Merchant","tier":"premium","value":"65000.00","as_of":"deterministic regtest fixture; not a live market quote","report":"Bitcoin demo exchange-rate report unlocked by a BTC Lightning payment.","resource":path}))).into_response();
    }
    let amount = if path == "/premium/report" { 3 } else { 50 };
    match merchant.challenge(path, amount).await {
        Ok(c) => {
            let header = format!(
                "L402 macaroon=\"{}\", invoice=\"{}\"",
                c.macaroon, c.invoice.invoice
            );
            (
                StatusCode::PAYMENT_REQUIRED,
                [
                    (header::WWW_AUTHENTICATE, header),
                    (header::CACHE_CONTROL, "no-store".into()),
                ],
                Json(L402Challenge {
                    resource: path.into(),
                    amount_sats: c.invoice.amount_sats,
                    invoice: c.invoice.invoice,
                }),
            )
                .into_response()
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "Merchant invoice unavailable").into_response(),
    }
}

#[cfg(test)]
#[path = "l402_tests.rs"]
mod tests;
