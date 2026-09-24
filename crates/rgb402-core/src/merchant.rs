//! Merchant metadata is untrusted display data, never spending authority.
use crate::{wallet::PaymentRequest, AssetId, PaymentStatus};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MerchantProfile {
    pub enabled: bool,
    pub public_catalog: bool,
    pub merchant_id: String,
    pub display_name: String,
    pub accepted_assets: Vec<AssetId>,
    pub catalog: String,
    pub orders: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Product {
    pub id: String,
    pub name: String,
    pub amount: String,
    pub asset_id: AssetId,
    pub available: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderInput {
    pub product_id: String,
    pub quantity: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    AwaitingPayment,
    Paid,
    Expired,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Order {
    pub id: String,
    pub merchant_id: String,
    pub product: Product,
    pub quantity: u64,
    pub payment: PaymentRequest,
    pub status: OrderStatus,
    pub payment_status: PaymentStatus,
    pub created_at: u64,
}

/// Owner-controlled settings. Identity, routes and payment authority are not editable.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MerchantSettings {
    pub enabled: bool,
    pub public_catalog: bool,
    pub display_name: String,
    pub accepted_assets: Vec<AssetId>,
    pub products: Vec<Product>,
}
