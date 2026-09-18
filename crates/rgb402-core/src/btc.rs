//! Human BTC Lightning transfers have their own limits and always require approval.
use crate::wallet::PolicyDecision;
#[derive(Clone, Debug)]
pub struct BtcTransferPolicy {
    pub max_payment_sats: u64,
    pub max_daily_sats: u64,
}
impl BtcTransferPolicy {
    pub fn evaluate(&self, amount: u64, available: u64, spent: u64) -> PolicyDecision {
        let reason = if amount == 0 || amount.checked_mul(1000).is_none() {
            Some("positive whole-satoshi amount required")
        } else if amount > self.max_payment_sats {
            Some("human BTC payment limit exceeded")
        } else if spent
            .checked_add(amount)
            .map_or(true, |total| total > self.max_daily_sats)
        {
            Some("human BTC daily budget exceeded")
        } else if amount > available {
            Some("no usable BTC channel supports this amount within its minimum and available capacity")
        } else {
            None
        };
        match reason {
            Some(reason) => PolicyDecision::Deny {
                reason: reason.into(),
            },
            None => PolicyDecision::RequireApproval {
                reason: "human BTC transfers always require application approval".into(),
            },
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn human_btc_never_auto_approves_and_budgets_do_not_overflow() {
        let p = BtcTransferPolicy {
            max_payment_sats: 100,
            max_daily_sats: 500,
        };
        assert!(matches!(
            p.evaluate(1, 1000, 0),
            PolicyDecision::RequireApproval { .. }
        ));
        for (amount, available, spent) in [
            (0, 1000, 0),
            (101, 1000, 0),
            (5, 4, 0),
            (5, 1000, 496),
            (5, 1000, u64::MAX),
        ] {
            assert!(matches!(
                p.evaluate(amount, available, spent),
                PolicyDecision::Deny { .. }
            ))
        }
    }
}
