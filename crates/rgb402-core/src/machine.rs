//! BTC machine purchases have an independent, integer-satoshi budget.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum MachineDecision {
    AllowAuto,
    RequireApproval,
    Deny { reason: String },
}

#[derive(Clone, Debug)]
pub struct MachinePolicy {
    pub auto_approve_below_sats: u64,
    pub max_single_payment_sats: u64,
    pub max_daily_spend_sats: u64,
}
impl MachinePolicy {
    /// The machine threshold is inclusive: configured 10 means up to 10 sats.
    pub fn evaluate(&self, amount: u64, available: u64, reserved_today: u64) -> MachineDecision {
        let reason = if amount == 0 {
            Some("fixed positive satoshi amount required")
        } else if amount > self.max_single_payment_sats {
            Some("machine single-payment limit exceeded")
        } else if amount > available {
            Some("insufficient outbound BTC Lightning balance")
        } else if reserved_today
            .checked_add(amount)
            .map_or(true, |sum| sum > self.max_daily_spend_sats)
        {
            Some("machine daily-spend limit exceeded")
        } else {
            None
        };
        if let Some(reason) = reason {
            MachineDecision::Deny {
                reason: reason.into(),
            }
        } else if amount <= self.auto_approve_below_sats {
            MachineDecision::AllowAuto
        } else {
            MachineDecision::RequireApproval
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_machine_limits_and_inclusive_threshold() {
        let p = MachinePolicy {
            auto_approve_below_sats: 10,
            max_single_payment_sats: 100,
            max_daily_spend_sats: 500,
        };
        for amount in [3, 10] {
            assert_eq!(p.evaluate(amount, 1000, 0), MachineDecision::AllowAuto);
        }
        assert_eq!(p.evaluate(50, 1000, 0), MachineDecision::RequireApproval);
        for (amount, balance, spent) in [
            (0, 1000, 0),
            (101, 1000, 0),
            (3, 2, 0),
            (3, 1000, 498),
            (3, 1000, u64::MAX),
        ] {
            assert!(matches!(
                p.evaluate(amount, balance, spent),
                MachineDecision::Deny { .. }
            ));
        }
    }
}
