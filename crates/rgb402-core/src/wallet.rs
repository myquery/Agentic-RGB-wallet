//! Wallet domain and deterministic policy. Amounts are integer asset base units.
use crate::{AssetId, PaymentId};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaymentRequest {
    pub asset_id: AssetId,
    pub amount: u64,
    pub invoice: String,
    pub payment_hash: PaymentId,
    pub expires_at: u64,
    pub network: String,
    pub carrier_msat: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub asset_id: AssetId,
    pub name: String,
    pub ticker: String,
    pub precision: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletBalance {
    pub asset_id: AssetId,
    pub onchain_spendable: u64,
    pub offchain_outbound: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PolicyDecision {
    Allow,
    RequireApproval { reason: String },
    Deny { reason: String },
}
#[derive(Clone, Debug)]
pub struct WalletPolicy {
    pub auto_approve_below: u64,
    pub max_single_payment: u64,
    pub max_daily_spend: u64,
    pub allowed_assets: HashSet<AssetId>,
    pub max_carrier_msat: u64,
}
impl WalletPolicy {
    pub fn evaluate(
        &self,
        request: &PaymentRequest,
        balance: u64,
        spent: u64,
        now: u64,
    ) -> PolicyDecision {
        let reason = if !self.allowed_assets.contains(&request.asset_id) {
            Some("asset is not allowed")
        } else if request.amount == 0 {
            Some("amount must be positive")
        } else if request.expires_at <= now {
            Some("invoice expired")
        } else if request.network != "Regtest" {
            Some("only regtest payments are enabled")
        } else if request.carrier_msat > self.max_carrier_msat {
            Some("Lightning carrier amount exceeds limit")
        } else if request.amount > balance {
            Some("insufficient outbound RGB balance")
        } else if request.amount > self.max_single_payment {
            Some("single payment limit exceeded")
        } else if request
            .amount
            .checked_add(spent)
            .map_or(true, |v| v > self.max_daily_spend)
        {
            Some("daily spend limit exceeded")
        } else {
            None
        };
        if let Some(reason) = reason {
            PolicyDecision::Deny {
                reason: reason.into(),
            }
        } else if request.amount >= self.auto_approve_below {
            PolicyDecision::RequireApproval {
                reason: "amount meets or exceeds auto approval threshold".into(),
            }
        } else {
            PolicyDecision::Allow
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (WalletPolicy, PaymentRequest) {
        let asset = AssetId::new("rgb:demo").unwrap();
        (
            WalletPolicy {
                auto_approve_below: 10,
                max_single_payment: 50,
                max_daily_spend: 100,
                allowed_assets: HashSet::from([asset.clone()]),
                max_carrier_msat: 1000,
            },
            PaymentRequest {
                asset_id: asset,
                amount: 5,
                invoice: "lnbcrt".into(),
                payment_hash: PaymentId::new("hash").unwrap(),
                expires_at: 200,
                network: "Regtest".into(),
                carrier_msat: 1000,
            },
        )
    }
    #[test]
    fn allowed_small_payment() {
        let (p, r) = setup();
        assert_eq!(p.evaluate(&r, 100, 0, 100), PolicyDecision::Allow);
    }
    #[test]
    fn approval_at_threshold() {
        let (p, mut r) = setup();
        r.amount = 10;
        assert!(matches!(
            p.evaluate(&r, 100, 0, 100),
            PolicyDecision::RequireApproval { .. }
        ));
    }
    #[test]
    fn policy_denials() {
        for case in [
            "asset", "balance", "single", "daily", "zero", "expired", "network", "carrier",
            "overflow",
        ] {
            let (mut p, mut r) = setup();
            let mut balance = 100;
            let mut spent = 0;
            match case {
                "asset" => p.allowed_assets.clear(),
                "balance" => balance = 0,
                "single" => r.amount = 51,
                "daily" => spent = 99,
                "zero" => r.amount = 0,
                "expired" => r.expires_at = 100,
                "network" => r.network = "Bitcoin".into(),
                "carrier" => r.carrier_msat = 1001,
                "overflow" => spent = u64::MAX,
                _ => unreachable!(),
            }
            assert!(
                matches!(
                    p.evaluate(&r, balance, spent, 100),
                    PolicyDecision::Deny { .. }
                ),
                "{case}"
            );
        }
    }
    #[test]
    fn negative_amount_is_not_representable() {
        assert!("-1".parse::<u64>().is_err());
    }
}
