//! Narrow client for RGB-Tools/rgb-lightning-node, see docs/node-api.md.
use crate::wallet::{ApprovedPayment, PaymentResult, WalletError};
use async_trait::async_trait;
use reqwest::{header, Client, Url};
use rgb402_core::{
    wallet::{Asset, PaymentRequest, WalletBalance},
    AssetId, PaymentId, PaymentStatus,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;

/// Node-native invoice issuance; no outgoing authorization capability.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateInvoice {
    pub asset_id: AssetId,
    pub asset_amount: u64,
    pub amt_msat: u64,
    pub expiry_sec: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description_hash: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct CreatedInvoice {
    pub invoice: String,
}
/// Explicit display allowlist. Node secrets and peer identities are never retained.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodePayment {
    pub payment_hash: PaymentId,
    pub inbound: bool,
    pub status: NodePaymentStatus,
    pub asset_id: Option<AssetId>,
    pub asset_amount: Option<u64>,
    pub amt_msat: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodePaymentStatus {
    Pending,
    Succeeded,
    Failed,
}
impl From<NodePaymentStatus> for PaymentStatus {
    fn from(s: NodePaymentStatus) -> Self {
        match s {
            NodePaymentStatus::Pending => Self::Pending,
            NodePaymentStatus::Succeeded => Self::Settled,
            NodePaymentStatus::Failed => Self::Failed,
        }
    }
}
#[async_trait]
pub trait RgbNode: Send + Sync {
    async fn create_invoice(&self, _: &CreateInvoice) -> Result<CreatedInvoice, WalletError> {
        Err(WalletError::Node("invoice creation unavailable"))
    }
    async fn list_payments(&self) -> Result<Vec<NodePayment>, WalletError> {
        Err(WalletError::Node("node history unavailable"))
    }

    async fn list_assets(&self) -> Result<Vec<Asset>, WalletError>;
    async fn asset_balance(&self, asset: &AssetId) -> Result<WalletBalance, WalletError>;
    async fn decode_invoice(&self, invoice: &str) -> Result<PaymentRequest, WalletError>;
    async fn send_payment(&self, payment: &ApprovedPayment) -> Result<PaymentResult, WalletError>;
    async fn payment_status(&self, hash: &PaymentId) -> Result<PaymentStatus, WalletError>;
}
pub struct RgbLightningClient {
    pub(crate) http: Client,
    pub(crate) base: Url,
}
impl RgbLightningClient {
    pub fn new(base: &str, token: Option<&str>) -> Result<Self, WalletError> {
        let base = Url::parse(base).map_err(|_| WalletError::Config("invalid node URL".into()))?;
        if !matches!(base.scheme(), "http" | "https")
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
        {
            return Err(WalletError::Config(
                "node URL must be an HTTP(S) origin without embedded credentials".into(),
            ));
        }
        let mut headers = header::HeaderMap::new();
        if let Some(token) = token {
            let mut value = header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| WalletError::Config("invalid node token".into()))?;
            value.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, value);
        }
        let http = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| WalletError::Node("client initialization failed"))?;
        Ok(Self { http, base })
    }
    pub(crate) async fn post<T: Serialize + Sync, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R, WalletError> {
        let url = self
            .base
            .join(path)
            .map_err(|_| WalletError::Node("invalid endpoint"))?;
        let response =
            self.http.post(url).json(body).send().await.map_err(|_| {
                WalletError::Node("node transport failed; submission may be uncertain")
            })?;
        if !response.status().is_success() {
            return Err(WalletError::Http(response.status().as_u16()));
        }
        response
            .json()
            .await
            .map_err(|_| WalletError::Node("invalid node response"))
    }
}
#[derive(Serialize)]
struct Invoice<'a> {
    invoice: &'a str,
}
#[derive(Serialize)]
struct AssetQuery<'a> {
    asset_id: &'a AssetId,
}
#[derive(Serialize)]
struct HashQuery<'a> {
    payment_hash: &'a PaymentId,
}
#[derive(Deserialize)]
struct BalanceDto {
    spendable: u64,
    offchain_outbound: u64,
}
#[derive(Deserialize)]
struct AssetsDto {
    nia: Option<Vec<Asset>>,
}
#[derive(Deserialize)]
struct LnInvoiceDto {
    asset_id: Option<AssetId>,
    asset_amount: Option<u64>,
    amt_msat: Option<u64>,
    timestamp: u64,
    expiry_sec: u64,
    payment_hash: PaymentId,
    network: String,
}
#[derive(Deserialize)]
enum StatusDto {
    Pending,
    Succeeded,
    Failed,
}
impl From<StatusDto> for PaymentStatus {
    fn from(s: StatusDto) -> Self {
        match s {
            StatusDto::Pending => Self::Pending,
            StatusDto::Succeeded => Self::Settled,
            StatusDto::Failed => Self::Failed,
        }
    }
}
#[derive(Deserialize)]
struct SendDto {
    payment_id: PaymentId,
    payment_hash: Option<PaymentId>,
    status: StatusDto,
}
#[derive(Deserialize)]
struct PaymentDto {
    payment: PaymentStateDto,
}
#[derive(Deserialize)]
struct PaymentStateDto {
    status: StatusDto,
}
#[async_trait]
impl RgbNode for RgbLightningClient {
    async fn create_invoice(&self, request: &CreateInvoice) -> Result<CreatedInvoice, WalletError> {
        if request.asset_amount == 0
            || request.amt_msat == 0
            || request.expiry_sec == 0
            || (request.description.is_some() && request.description_hash.is_some())
        {
            return Err(WalletError::Invalid("invalid invoice request"));
        }
        self.post("lninvoice", request).await
    }
    async fn list_payments(&self) -> Result<Vec<NodePayment>, WalletError> {
        #[derive(Deserialize)]
        struct Payments {
            payments: Vec<NodePayment>,
        }
        let response = self
            .http
            .get(
                self.base
                    .join("listpayments")
                    .map_err(|_| WalletError::Node("invalid endpoint"))?,
            )
            .send()
            .await
            .map_err(|_| WalletError::Node("history unavailable"))?;
        if !response.status().is_success() {
            return Err(WalletError::Http(response.status().as_u16()));
        }
        Ok(response
            .json::<Payments>()
            .await
            .map_err(|_| WalletError::Node("invalid history response"))?
            .payments)
    }

    async fn list_assets(&self) -> Result<Vec<Asset>, WalletError> {
        #[derive(Serialize)]
        struct Filter {
            filter_asset_schemas: [&'static str; 1],
        }
        let result: AssetsDto = self
            .post(
                "listassets",
                &Filter {
                    filter_asset_schemas: ["Nia"],
                },
            )
            .await?;
        Ok(result.nia.unwrap_or_default())
    }
    async fn asset_balance(&self, asset: &AssetId) -> Result<WalletBalance, WalletError> {
        let result: BalanceDto = self
            .post("assetbalance", &AssetQuery { asset_id: asset })
            .await?;
        Ok(WalletBalance {
            asset_id: asset.clone(),
            onchain_spendable: result.spendable,
            offchain_outbound: result.offchain_outbound,
        })
    }
    async fn decode_invoice(&self, invoice: &str) -> Result<PaymentRequest, WalletError> {
        if !invoice.to_ascii_lowercase().starts_with("lnbcrt") {
            return Err(WalletError::Invalid("expected a regtest RGB Lightning invoice; on-chain RGB and ordinary BTC payments are outside milestone 1"));
        }
        let d: LnInvoiceDto = self.post("decodelninvoice", &Invoice { invoice }).await?;
        Ok(PaymentRequest {
            asset_id: d
                .asset_id
                .ok_or(WalletError::Invalid("invoice has no RGB asset"))?,
            amount: d
                .asset_amount
                .ok_or(WalletError::Invalid("invoice has no fixed RGB amount"))?,
            carrier_msat: d.amt_msat.ok_or(WalletError::Invalid(
                "invoice has no fixed Lightning amount",
            ))?,
            invoice: invoice.into(),
            payment_hash: d.payment_hash,
            network: d.network,
            expires_at: d
                .timestamp
                .checked_add(d.expiry_sec)
                .ok_or(WalletError::Invalid("expiry overflow"))?,
        })
    }
    async fn send_payment(&self, payment: &ApprovedPayment) -> Result<PaymentResult, WalletError> {
        let d: SendDto = self
            .post(
                "sendpayment",
                &Invoice {
                    invoice: &payment.request().invoice,
                },
            )
            .await?;
        let hash = d
            .payment_hash
            .unwrap_or_else(|| payment.request().payment_hash.clone());
        if hash != payment.request().payment_hash {
            return Err(WalletError::Node("returned payment hash mismatch"));
        }
        Ok(PaymentResult {
            payment_id: d.payment_id,
            payment_hash: hash,
            status: d.status.into(),
        })
    }
    async fn payment_status(&self, hash: &PaymentId) -> Result<PaymentStatus, WalletError> {
        let d: PaymentDto = self
            .post("getpayment", &HashQuery { payment_hash: hash })
            .await?;
        Ok(d.payment.status.into())
    }
}

#[cfg(test)]
mod receive_tests {
    use super::*;
    #[test]
    fn history_projection_drops_secrets_and_preserves_direction() {
        let node = serde_json::json!({"payment_hash":"hash","inbound":true,"status":"Succeeded","asset_id":"rgb:demo","asset_amount":5,"amt_msat":3000000,"created_at":1,"updated_at":2,"preimage":"secret-proof","payee_pubkey":"not-a-sender","description":"untrusted"});
        let payment: NodePayment = serde_json::from_value(node).unwrap();
        let safe = serde_json::to_value(payment).unwrap();
        assert_eq!(safe["inbound"], true);
        assert_eq!(safe["asset_amount"], 5);
        assert!(safe.get("preimage").is_none());
        assert!(safe.get("payee_pubkey").is_none());
    }
    #[tokio::test]
    async fn invoice_and_history_http_contract() {
        use axum::{
            routing::{get, post},
            Json, Router,
        };
        let app = Router::new().route("/lninvoice",post(|Json(body):Json<serde_json::Value>| async move {
            assert_eq!(body["asset_amount"],5); assert_eq!(body["amt_msat"],3000000); assert_eq!(body["expiry_sec"],3600); assert_eq!(body["asset_id"],"rgb:demo");
            Json(serde_json::json!({"invoice":"test-invoice"}))
        })).route("/listpayments",get(||async { Json(serde_json::json!({"payments":[{"payment_hash":"hash","inbound":true,"status":"Succeeded","asset_id":"rgb:demo","asset_amount":5,"amt_msat":3000000,"created_at":1,"updated_at":2,"preimage":"secret"}]})) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = RgbLightningClient::new(&format!("http://{addr}"), None).unwrap();
        let r = CreateInvoice {
            asset_id: AssetId::new("rgb:demo").unwrap(),
            asset_amount: 5,
            amt_msat: 3000000,
            expiry_sec: 3600,
            description: None,
            description_hash: None,
        };
        assert_eq!(
            client.create_invoice(&r).await.unwrap().invoice,
            "test-invoice"
        );
        assert_eq!(client.list_payments().await.unwrap().len(), 1);
        task.abort();
    }
}
