use crate::wallet::WalletError;
use rgb402_core::{wallet::WalletPolicy, AssetId};
use std::{collections::HashSet, env, path::PathBuf};

// Deliberately no Debug implementation: tokens must not enter logs.
pub struct WalletConfig {
    pub node_url: String,
    pub node_token: Option<String>,
    pub policy: WalletPolicy,
    pub state_path: PathBuf,
}
impl WalletConfig {
    pub fn from_env() -> Result<Self, WalletError> {
        let required =
            |name: &str| env::var(name).map_err(|_| WalletError::Config(format!("missing {name}")));
        let number = |name: &str| -> Result<u64, WalletError> {
            required(name)?.parse().map_err(|_| {
                WalletError::Config(format!("{name} must be an unsigned integer in base units"))
            })
        };
        let allowed_assets: HashSet<_> = required("ALLOWED_ASSET_IDS")?
            .split(',')
            .map(|s| AssetId::new(s.trim()))
            .collect::<Result<_, _>>()?;
        let policy = WalletPolicy {
            auto_approve_below: number("AUTO_APPROVE_BELOW")?,
            max_single_payment: number("MAX_SINGLE_PAYMENT")?,
            max_daily_spend: number("MAX_DAILY_SPEND")?,
            max_carrier_msat: number("MAX_CARRIER_MSAT")?,
            allowed_assets,
        };
        if policy.max_single_payment == 0
            || policy.max_single_payment > policy.max_daily_spend
            || policy.auto_approve_below > policy.max_single_payment
        {
            return Err(WalletError::Config(
                "require 0 < single <= daily and auto <= single".into(),
            ));
        }
        Ok(Self {
            node_url: required("RGB_NODE_URL")?,
            node_token: env::var("RGB_NODE_TOKEN").ok().filter(|s| !s.is_empty()),
            state_path: env::var("WALLET_STATE_PATH")
                .unwrap_or_else(|_| ".wallet-state.json".into())
                .into(),
            policy,
        })
    }
}
