//! BTC-only Lightning capabilities. Proofs deliberately have no Debug/Serialize implementation.
use crate::{
    rgb::RgbLightningClient,
    wallet::{PaymentResult, WalletError},
};
use async_trait::async_trait;
use rgb402_core::{PaymentId, PaymentStatus};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LightningInvoice {
    pub invoice: String,
    pub payment_hash: PaymentId,
    pub amount_sats: u64,
    pub expires_at: u64,
}
/// Only deterministic payment services in this crate can construct execution authority.
pub struct ApprovedLightningPayment(pub(crate) LightningInvoice);
pub use ApprovedLightningPayment as ApprovedMachinePayment;
impl ApprovedLightningPayment {
    pub fn invoice(&self) -> &LightningInvoice {
        &self.0
    }
}
pub struct LightningPayment {
    pub status: PaymentStatus,
    preimage: Option<String>,
}
impl LightningPayment {
    pub fn new(status: PaymentStatus, preimage: Option<String>) -> Self {
        Self { status, preimage }
    }
    pub(crate) fn proof(&self, hash: &PaymentId) -> Result<&str, WalletError> {
        if self.status != PaymentStatus::Settled {
            return Err(WalletError::Invalid("payment not settled"));
        }
        let proof = self
            .preimage
            .as_deref()
            .ok_or(WalletError::Node("settled payment has no proof"))?;
        let bytes = hex::decode(proof).map_err(|_| WalletError::Node("invalid payment proof"))?;
        if bytes.len() != 32 || hex::encode(Sha256::digest(&bytes)) != hash.as_str() {
            return Err(WalletError::Node("payment proof does not match invoice"));
        }
        Ok(proof)
    }
}
#[async_trait]
pub trait LightningNode: Send + Sync {
    async fn create_invoice(
        &self,
        amount_sats: u64,
        expiry_sec: u32,
    ) -> Result<LightningInvoice, WalletError>;
    async fn decode_btc_invoice(&self, invoice: &str) -> Result<LightningInvoice, WalletError>;
    async fn outbound_sats(&self) -> Result<u64, WalletError>;
    /// Capacity on a usable channel whose negotiated minimum permits this payment.
    async fn outbound_sats_for(&self, _amount_sats: u64) -> Result<u64, WalletError> {
        self.outbound_sats().await
    }
    async fn send_btc(
        &self,
        payment: &ApprovedMachinePayment,
    ) -> Result<PaymentResult, WalletError>;
    async fn btc_payment(&self, hash: &PaymentId) -> Result<LightningPayment, WalletError>;
}
#[derive(Deserialize)]
enum Status {
    Pending,
    Succeeded,
    Failed,
}
impl From<Status> for PaymentStatus {
    fn from(s: Status) -> Self {
        match s {
            Status::Pending => Self::Pending,
            Status::Succeeded => Self::Settled,
            Status::Failed => Self::Failed,
        }
    }
}
#[async_trait]
impl LightningNode for RgbLightningClient {
    async fn create_invoice(
        &self,
        amount_sats: u64,
        expiry_sec: u32,
    ) -> Result<LightningInvoice, WalletError> {
        #[derive(Deserialize)]
        struct Created {
            invoice: String,
        }
        let amount = amount_sats
            .checked_mul(1000)
            .filter(|v| *v > 0)
            .ok_or(WalletError::Invalid("invalid satoshi amount"))?;
        let d: Created = self.post("lninvoice", &serde_json::json!({"amt_msat":amount,"expiry_sec":expiry_sec,"description":"RGB402 premium demo report"})).await?;
        let decoded = self.decode_btc_invoice(&d.invoice).await?;
        if decoded.amount_sats != amount_sats {
            return Err(WalletError::Node("issued invoice amount mismatch"));
        }
        Ok(decoded)
    }
    async fn decode_btc_invoice(&self, invoice: &str) -> Result<LightningInvoice, WalletError> {
        if invoice.len() > 8192 || !invoice.starts_with("lnbcrt") {
            return Err(WalletError::Invalid("expected regtest Lightning invoice"));
        }
        #[derive(Deserialize)]
        struct Decoded {
            asset_id: Option<String>,
            asset_amount: Option<u64>,
            amt_msat: Option<u64>,
            timestamp: u64,
            expiry_sec: u64,
            payment_hash: PaymentId,
            network: String,
        }
        let d: Decoded = self
            .post("decodelninvoice", &serde_json::json!({"invoice":invoice}))
            .await?;
        let amount = d
            .amt_msat
            .filter(|v| *v > 0 && v % 1000 == 0)
            .ok_or(WalletError::Invalid("fixed whole-satoshi invoice required"))?;
        if d.asset_id.is_some() || d.asset_amount.is_some() || d.network != "Regtest" {
            return Err(WalletError::Invalid(
                "L402 requires BTC-only regtest invoice",
            ));
        }
        if hex::decode(d.payment_hash.as_str()).map_or(true, |h| h.len() != 32) {
            return Err(WalletError::Invalid("invalid payment hash"));
        }
        Ok(LightningInvoice {
            invoice: invoice.into(),
            payment_hash: d.payment_hash,
            amount_sats: amount / 1000,
            expires_at: d
                .timestamp
                .checked_add(d.expiry_sec)
                .ok_or(WalletError::Invalid("expiry overflow"))?,
        })
    }
    async fn outbound_sats(&self) -> Result<u64, WalletError> {
        self.channel_capacity(None).await
    }
    async fn outbound_sats_for(&self, amount_sats: u64) -> Result<u64, WalletError> {
        self.channel_capacity(Some(amount_sats)).await
    }
    async fn send_btc(
        &self,
        payment: &ApprovedMachinePayment,
    ) -> Result<PaymentResult, WalletError> {
        #[derive(Deserialize)]
        struct Sent {
            payment_id: PaymentId,
            payment_hash: Option<PaymentId>,
            status: Status,
        }
        let d: Sent = self
            .post(
                "sendpayment",
                &serde_json::json!({"invoice":payment.0.invoice}),
            )
            .await?;
        if d.payment_hash
            .as_ref()
            .is_some_and(|h| h != &payment.0.payment_hash)
        {
            return Err(WalletError::Node("returned payment hash mismatch"));
        }
        Ok(PaymentResult {
            payment_id: d.payment_id,
            payment_hash: payment.0.payment_hash.clone(),
            status: d.status.into(),
        })
    }
    async fn btc_payment(&self, hash: &PaymentId) -> Result<LightningPayment, WalletError> {
        #[derive(Deserialize)]
        struct ResultDto {
            payment: State,
        }
        #[derive(Deserialize)]
        struct State {
            status: Status,
            preimage: Option<String>,
            payment_hash: PaymentId,
            asset_id: Option<String>,
            asset_amount: Option<u64>,
        }
        let d: ResultDto = self
            .post("getpayment", &serde_json::json!({"payment_hash":hash}))
            .await?;
        if d.payment.payment_hash != *hash
            || d.payment.asset_id.is_some()
            || d.payment.asset_amount.is_some()
        {
            return Err(WalletError::Node("payment identity mismatch"));
        }
        Ok(LightningPayment::new(
            d.payment.status.into(),
            d.payment.preimage,
        ))
    }
}

impl RgbLightningClient {
    async fn channel_capacity(&self, amount_sats: Option<u64>) -> Result<u64, WalletError> {
        #[derive(Deserialize)]
        struct Channels {
            channels: Vec<Channel>,
        }
        #[derive(Deserialize)]
        struct Channel {
            is_usable: bool,
            outbound_balance_msat: u64,
            next_outbound_htlc_limit_msat: u64,
            next_outbound_htlc_minimum_msat: Option<u64>,
        }
        let response = self
            .http
            .get(
                self.base
                    .join("listchannels")
                    .map_err(|_| WalletError::Node("invalid endpoint"))?,
            )
            .send()
            .await
            .map_err(|_| WalletError::Node("balance transport failed"))?;
        if !response.status().is_success() {
            return Err(WalletError::Http(response.status().as_u16()));
        }
        let d: Channels = response
            .json()
            .await
            .map_err(|_| WalletError::Node("invalid channel balance"))?;
        // The wallet balance is the sum of usable local channel balances. The
        // next-HTLC limit is a routing constraint and can be lower than that
        // balance. A payment still has to fit one eligible channel in this
        // direct-channel demo, so preparation asks for the largest eligible
        // routing limit below.
        if amount_sats.is_none() {
            return d
                .channels
                .iter()
                .filter(|c| c.is_usable)
                .map(|c| c.outbound_balance_msat / 1000)
                .try_fold(0u64, |total, sats| {
                    total
                        .checked_add(sats)
                        .ok_or(WalletError::Node("channel balance overflow"))
                });
        }
        let amount = amount_sats.expect("checked above");
        // A single payment must fit a usable channel; conservative for this direct-channel demo.
        Ok(d.channels
            .iter()
            .filter(|c| {
                c.is_usable
                    && amount.checked_mul(1000).is_some_and(|msat| {
                        c.next_outbound_htlc_minimum_msat
                            .is_some_and(|minimum| msat >= minimum)
                            && msat <= c.next_outbound_htlc_limit_msat
                    })
            })
            .map(|c| c.next_outbound_htlc_limit_msat / 1000)
            .max()
            .unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{OriginalUri, State},
        http::HeaderMap,
        routing::{get, post},
        Json, Router,
    };
    use std::sync::{Arc, Mutex};
    #[tokio::test]
    async fn btc_node_contract_uses_invoice_only_payload_and_private_preimage() {
        let seen = Arc::new(Mutex::new(vec![]));
        async fn handler(
            State(seen): State<Arc<Mutex<Vec<String>>>>,
            OriginalUri(uri): OriginalUri,
            headers: HeaderMap,
            Json(body): Json<serde_json::Value>,
        ) -> Json<serde_json::Value> {
            assert_eq!(headers["authorization"], "Bearer test-only");
            seen.lock().unwrap().push(uri.path().into());
            let hash = hex::encode(Sha256::digest([7; 32]));
            Json(match uri.path() {
                "/lninvoice" => {
                    assert_eq!(body["amt_msat"], 3000);
                    assert!(body.get("asset_id").is_none());
                    serde_json::json!({"invoice":"lnbcrt3"})
                }
                "/decodelninvoice" => {
                    serde_json::json!({"asset_id":null,"asset_amount":null,"amt_msat":3000,"timestamp":1,"expiry_sec":100,"network":"Regtest","payment_hash":hash})
                }
                "/sendpayment" => {
                    assert_eq!(body, serde_json::json!({"invoice":"lnbcrt3"}));
                    serde_json::json!({"payment_id":hash,"payment_hash":hash,"status":"Pending"})
                }
                "/getpayment" => {
                    serde_json::json!({"payment":{"payment_hash":hash,"status":"Succeeded","preimage":hex::encode([7;32]),"asset_id":null,"asset_amount":null}})
                }
                _ => unreachable!(),
            })
        }
        let app=Router::new().route("/lninvoice",post(handler)).route("/decodelninvoice",post(handler)).route("/sendpayment",post(handler)).route("/getpayment",post(handler)).route("/listchannels",get(||async{Json(serde_json::json!({"channels":[{"is_usable":true,"outbound_balance_msat":900000,"next_outbound_htlc_limit_msat":1000000,"next_outbound_htlc_minimum_msat":3000},{"is_usable":true,"outbound_balance_msat":375000,"next_outbound_htlc_limit_msat":400000,"next_outbound_htlc_minimum_msat":3000},{"is_usable":false,"outbound_balance_msat":900000,"next_outbound_htlc_limit_msat":900000,"next_outbound_htlc_minimum_msat":3000}]}))})).with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let node = RgbLightningClient::new(&url, Some("test-only")).unwrap();
        let invoice = node.create_invoice(3, 100).await.unwrap();
        assert_eq!(invoice.amount_sats, 3);
        assert_eq!(node.outbound_sats().await.unwrap(), 1275);
        assert_eq!(node.outbound_sats_for(2).await.unwrap(), 0);
        assert_eq!(node.outbound_sats_for(3).await.unwrap(), 1000);
        assert_eq!(node.outbound_sats_for(1001).await.unwrap(), 0);
        node.send_btc(&ApprovedMachinePayment(invoice.clone()))
            .await
            .unwrap();
        let state = node.btc_payment(&invoice.payment_hash).await.unwrap();
        assert!(state.proof(&invoice.payment_hash).is_ok());
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                "/lninvoice",
                "/decodelninvoice",
                "/sendpayment",
                "/getpayment"
            ]
        );
        server.abort();
    }
    #[tokio::test]
    async fn btc_decoder_rejects_rgb_fractional_and_wrong_network_invoices() {
        for (asset, msat, network) in [
            (Some("rgb:demo"), 3000, "Regtest"),
            (None, 3001, "Regtest"),
            (None, 0, "Regtest"),
            (None, 3000, "Bitcoin"),
        ] {
            let response = serde_json::json!({"asset_id":asset,"asset_amount":null,"amt_msat":msat,"timestamp":1,"expiry_sec":100,"network":network,"payment_hash":hex::encode([7;32])});
            let app = Router::new().route(
                "/decodelninvoice",
                post(move || {
                    let response = response.clone();
                    async move { Json(response) }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            assert!(RgbLightningClient::new(&url, None)
                .unwrap()
                .decode_btc_invoice("lnbcrt3")
                .await
                .is_err());
            server.abort();
        }
    }
}
