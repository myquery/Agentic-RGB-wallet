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

#[async_trait]
pub trait RgbNode: Send + Sync {
    async fn list_assets(&self) -> Result<Vec<Asset>, WalletError>;
    async fn asset_balance(&self, asset: &AssetId) -> Result<WalletBalance, WalletError>;
    async fn decode_invoice(&self, invoice: &str) -> Result<PaymentRequest, WalletError>;
    async fn send_payment(&self, payment: &ApprovedPayment) -> Result<PaymentResult, WalletError>;
    async fn payment_status(&self, hash: &PaymentId) -> Result<PaymentStatus, WalletError>;
}
pub struct RgbLightningClient {
    http: Client,
    base: Url,
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
    async fn post<T: Serialize + Sync, R: DeserializeOwned>(
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
