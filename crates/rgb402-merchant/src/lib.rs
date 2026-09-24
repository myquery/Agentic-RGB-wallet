use axum::extract::State;
pub mod store;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rgb402_core::{
    AssetId, PaymentAmount, PaymentId, PaymentRequest, PaymentRequestId, RGB402_PAYMENT_ID_HEADER,
};
use rgb402_core::{PaymentChallenge, UnixTimestamp};
use rgb402_payment::SettlementVerifier;
use serde::Serialize;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Clone, Debug)]
pub struct MerchantConfig {
    pub resource_path: String,
    pub payment_request_id: PaymentRequestId,
    pub asset_id: AssetId,
    pub amount: PaymentAmount,
    pub invoice: String,
    pub expires_at: UnixTimestamp,
    pub premium_body: PremiumAnalysis,
}

impl MerchantConfig {
    pub fn premium_analysis_default() -> Self {
        Self {
            resource_path: "/premium-analysis".to_owned(),
            payment_request_id: PaymentRequestId::new("req-premium-analysis")
                .expect("static request id is valid"),
            asset_id: AssetId::boss(),
            amount: PaymentAmount::from_major_minor(4, 0, 2)
                .expect("static amount precision is valid"),
            invoice: "simulated-rgb-ln-invoice:req-premium-analysis".to_owned(),
            expires_at: UnixTimestamp::new(4_102_444_800),
            premium_body: PremiumAnalysis::default(),
        }
    }

    pub fn payment_request(&self) -> PaymentRequest {
        PaymentRequest {
            payment_request_id: self.payment_request_id.clone(),
            asset_id: self.asset_id.clone(),
            amount: self.amount,
            invoice: self.invoice.clone(),
            expires_at: self.expires_at,
            resource: self.resource_path.clone(),
        }
    }

    pub fn challenge(&self) -> PaymentChallenge {
        self.payment_request().challenge()
    }
}

#[derive(Clone)]
pub struct MerchantState {
    config: Arc<MerchantConfig>,
    settlement: Arc<dyn SettlementVerifier>,
}

pub fn router(config: MerchantConfig, settlement: Arc<dyn SettlementVerifier>) -> Router {
    let resource_path = config.resource_path.clone();
    let state = MerchantState {
        config: Arc::new(config),
        settlement,
    };

    Router::new()
        .route(&resource_path, get(premium_resource))
        .route("/healthz", get(healthz))
        .with_state(state)
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { ok: true })
}

async fn premium_resource(State(state): State<MerchantState>, headers: HeaderMap) -> Response {
    let expected_request = state.config.payment_request();

    if let Some(payment_id) = payment_id_from_headers(&headers) {
        match state
            .settlement
            .is_settled(&expected_request, &payment_id)
            .await
        {
            Ok(true) => {
                info!(%payment_id, "payment settled; unlocking premium resource");
                return (StatusCode::OK, Json(state.config.premium_body.clone())).into_response();
            }
            Ok(false) => {
                warn!(%payment_id, "payment id did not verify against settlement service");
            }
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: error.to_string(),
                    }),
                )
                    .into_response();
            }
        }
    }

    (StatusCode::PAYMENT_REQUIRED, Json(state.config.challenge())).into_response()
}

fn payment_id_from_headers(headers: &HeaderMap) -> Option<PaymentId> {
    headers
        .get(RGB402_PAYMENT_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| PaymentId::new(value.to_owned()).ok())
}

#[derive(Clone, Debug, Serialize)]
pub struct PremiumAnalysis {
    pub title: String,
    pub body: String,
}

impl Default for PremiumAnalysis {
    fn default() -> Self {
        Self {
            title: "BOSS market signal".to_owned(),
            body: "RGB402 bootstrap demo unlocked this deterministic premium analysis.".to_owned(),
        }
    }
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

pub mod l402;
