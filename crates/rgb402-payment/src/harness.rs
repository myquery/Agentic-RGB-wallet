//! Application-owned economic state. No model text or deserializable execution capability.
use crate::wallet::WalletError;
use rgb402_core::PaymentStatus;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    RgbPayment,
    BtcTransfer,
    L402Resource,
}
/// The full immutable parameters stay in the service plan/journal. This contract
/// binds them by digest and states which evidence is necessary for completion.
#[derive(Clone, Debug, Serialize)]
pub struct TaskContract {
    pub economic_action_id: String,
    pub kind: TaskKind,
    pub completion: Completion,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    NodeSettlement,
    SettledPaymentAndAuthenticatedResource,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Authorization {
    Automatic,
    Human,
    LegacyUnknown,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Requested,
    Decoded,
    Challenged,
    Validated,
    Prepared,
    AwaitingAuthorization,
    Authorized,
    Reserved,
    Submitted,
    Uncertain,
    Settled,
    ProofReady,
    Complete,
    Failed,
    Cancelled,
    Denied,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Proposed,
    Decoded,
    Challenged,
    Validated,
    Prepared,
    PolicyAutomatic,
    ApprovalRequired,
    PolicyDenied,
    HumanAuthorized,
    Reserved,
    SubmissionAttempted,
    RecoveredReservation,
    StatusPending,
    StatusUncertain,
    PaymentSettled,
    PaymentFailed,
    ProofVerified,
    ResourceRetry,
    ResourceUnlocked,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReservationAuthority {
    pub economic_action_id: String,
    pub source: Authorization,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub economic_action_id: String,
    pub kind: TaskKind,
    pub state: State,
    pub authorization: Option<Authorization>,
    /// Attempts observed in this process, not an invented count after a crash.
    pub observed_submission_attempts: u8,
    pub submission_may_have_occurred: bool,
    pub trace: Vec<Event>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Action {
    snapshot: TaskSnapshot,
}
impl Action {
    pub(crate) fn new(kind: TaskKind, contract: &impl Serialize) -> Result<Self, WalletError> {
        let bytes = serde_json::to_vec(&(kind, contract))?;
        Ok(Self {
            snapshot: TaskSnapshot {
                economic_action_id: hex::encode(Sha256::digest(bytes)),
                kind,
                state: State::Requested,
                authorization: None,
                observed_submission_attempts: 0,
                submission_may_have_occurred: false,
                trace: vec![Event::Proposed],
            },
        })
    }
    pub fn contract(&self) -> TaskContract {
        TaskContract {
            economic_action_id: self.snapshot.economic_action_id.clone(),
            kind: self.snapshot.kind,
            completion: match self.snapshot.kind {
                TaskKind::RgbPayment | TaskKind::BtcTransfer => Completion::NodeSettlement,
                TaskKind::L402Resource => Completion::SettledPaymentAndAuthenticatedResource,
            },
        }
    }
    pub fn snapshot(&self) -> TaskSnapshot {
        self.snapshot.clone()
    }
    pub(crate) fn decoded(&mut self) -> Result<(), WalletError> {
        self.step(
            State::Requested,
            if self.snapshot.kind != TaskKind::L402Resource {
                State::Decoded
            } else {
                State::Challenged
            },
            if self.snapshot.kind != TaskKind::L402Resource {
                Event::Decoded
            } else {
                Event::Challenged
            },
        )
    }
    pub(crate) fn validated(&mut self) -> Result<(), WalletError> {
        let from = if self.snapshot.kind != TaskKind::L402Resource {
            State::Decoded
        } else {
            State::Challenged
        };
        self.step(from, State::Validated, Event::Validated)?;
        self.step(State::Validated, State::Prepared, Event::Prepared)
    }
    pub(crate) fn policy(&mut self, automatic: bool, denied: bool) -> Result<(), WalletError> {
        if denied {
            return self.step(State::Prepared, State::Denied, Event::PolicyDenied);
        }
        if automatic {
            self.step(State::Prepared, State::Authorized, Event::PolicyAutomatic)?;
            self.snapshot.authorization = Some(Authorization::Automatic);
            Ok(())
        } else {
            self.step(
                State::Prepared,
                State::AwaitingAuthorization,
                Event::ApprovalRequired,
            )
        }
    }
    pub(crate) fn human(&mut self) -> Result<(), WalletError> {
        if self.snapshot.state == State::Authorized {
            self.snapshot.authorization = Some(Authorization::Human);
            self.event(Event::HumanAuthorized);
            return Ok(());
        }
        self.step(
            State::AwaitingAuthorization,
            State::Authorized,
            Event::HumanAuthorized,
        )?;
        self.snapshot.authorization = Some(Authorization::Human);
        Ok(())
    }
    pub(crate) fn cancel(&mut self) -> Result<(), WalletError> {
        if !matches!(
            self.snapshot.state,
            State::AwaitingAuthorization | State::Authorized
        ) {
            return Err(invalid());
        }
        self.snapshot.state = State::Cancelled;
        self.event(Event::Cancelled);
        Ok(())
    }
    pub(crate) fn authority(
        &self,
        contract: &impl Serialize,
    ) -> Result<ReservationAuthority, WalletError> {
        let actual = Self::new(self.snapshot.kind, contract)?;
        if self.snapshot.state != State::Authorized
            || actual.snapshot.economic_action_id != self.snapshot.economic_action_id
        {
            return Err(invalid());
        }
        Ok(ReservationAuthority {
            economic_action_id: self.snapshot.economic_action_id.clone(),
            source: self.snapshot.authorization.ok_or_else(invalid)?,
        })
    }
    /// Called only after the existing reservation journal was successfully flushed.
    pub(crate) fn reserved(&mut self) -> Result<(), WalletError> {
        self.step(State::Authorized, State::Reserved, Event::Reserved)
    }
    pub(crate) fn submit(&mut self) -> Result<(), WalletError> {
        self.step(
            State::Reserved,
            State::Submitted,
            Event::SubmissionAttempted,
        )?;
        self.snapshot.observed_submission_attempts = 1;
        self.snapshot.submission_may_have_occurred = true;
        Ok(())
    }
    pub(crate) fn recover(
        kind: TaskKind,
        contract: &impl Serialize,
        authority: Option<&ReservationAuthority>,
    ) -> Result<Self, WalletError> {
        let mut action = Self::new(kind, contract)?;
        if authority.is_some_and(|a| a.economic_action_id != action.snapshot.economic_action_id) {
            return Err(WalletError::Invalid("reservation contract mismatch"));
        }
        action.snapshot.authorization =
            Some(authority.map_or(Authorization::LegacyUnknown, |a| a.source));
        action.snapshot.state = State::Uncertain;
        action.snapshot.submission_may_have_occurred = true;
        action.event(Event::RecoveredReservation);
        Ok(action)
    }
    pub(crate) fn status(&mut self, status: Option<&PaymentStatus>) -> Result<(), WalletError> {
        if !self.snapshot.submission_may_have_occurred {
            return Err(invalid());
        }
        if matches!(
            self.snapshot.state,
            State::Settled | State::ProofReady | State::Complete
        ) {
            return if status == Some(&PaymentStatus::Settled) {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        if self.snapshot.state == State::Failed {
            return if status == Some(&PaymentStatus::Failed) {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        let (state, event) = match status {
            Some(PaymentStatus::Pending) => (State::Submitted, Event::StatusPending),
            Some(PaymentStatus::Settled) => (State::Settled, Event::PaymentSettled),
            Some(PaymentStatus::Failed) => (State::Failed, Event::PaymentFailed),
            None => (State::Uncertain, Event::StatusUncertain),
        };
        self.snapshot.state = state;
        self.event(event);
        Ok(())
    }
    pub(crate) fn proof(&mut self) -> Result<(), WalletError> {
        if self.snapshot.kind != TaskKind::L402Resource {
            return Err(invalid());
        }
        if matches!(self.snapshot.state, State::ProofReady | State::Complete) {
            return Ok(());
        }
        self.step(State::Settled, State::ProofReady, Event::ProofVerified)
    }
    pub(crate) fn retry_resource(&mut self) -> Result<(), WalletError> {
        if !matches!(self.snapshot.state, State::ProofReady | State::Complete) {
            return Err(invalid());
        }
        self.event(Event::ResourceRetry);
        Ok(())
    }
    pub(crate) fn unlocked(&mut self) -> Result<(), WalletError> {
        if self.snapshot.state == State::Complete {
            return Ok(());
        }
        self.step(State::ProofReady, State::Complete, Event::ResourceUnlocked)
    }
    fn step(&mut self, from: State, to: State, event: Event) -> Result<(), WalletError> {
        if self.snapshot.state != from {
            return Err(invalid());
        }
        self.snapshot.state = to;
        self.event(event);
        Ok(())
    }
    fn event(&mut self, event: Event) {
        if self.snapshot.trace.last() == Some(&event) {
            return;
        }
        if self.snapshot.trace.len() == 32 {
            self.snapshot.trace.remove(0);
        }
        self.snapshot.trace.push(event);
    }
}
fn invalid() -> WalletError {
    WalletError::Invalid("invalid economic task transition or authorization binding")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepared() -> Action {
        let mut a = Action::new(TaskKind::L402Resource, &("url", "invoice", 3)).unwrap();
        a.decoded().unwrap();
        a.validated().unwrap();
        a.policy(false, false).unwrap();
        a
    }
    #[test]
    fn authorization_binding_and_at_most_once_transitions() {
        let mut a = prepared();
        assert!(a.submit().is_err());
        assert!(a.reserved().is_err());
        assert!(a.proof().is_err());
        assert!(a.unlocked().is_err());
        a.human().unwrap();
        assert!(a.authority(&("changed", "invoice", 3)).is_err());
        assert!(a.authority(&("url", "changed", 3)).is_err());
        assert!(a.authority(&("url", "invoice", 50)).is_err());
        a.authority(&("url", "invoice", 3)).unwrap();
        a.reserved().unwrap();
        a.submit().unwrap();
        assert!(a.submit().is_err());
        a.status(Some(&PaymentStatus::Pending)).unwrap();
        assert!(a.retry_resource().is_err());
        a.status(Some(&PaymentStatus::Settled)).unwrap();
        a.proof().unwrap();
        a.retry_resource().unwrap();
        a.unlocked().unwrap();
        assert_eq!(a.snapshot().state, State::Complete);
        assert!(a.submit().is_err());
    }
    #[test]
    fn recovery_never_restores_execution_authority() {
        let mut a = prepared();
        a.human().unwrap();
        let authority = a.authority(&("url", "invoice", 3)).unwrap();
        let mut recovered = Action::recover(
            TaskKind::L402Resource,
            &("url", "invoice", 3),
            Some(&authority),
        )
        .unwrap();
        assert_eq!(
            recovered.snapshot().authorization,
            Some(Authorization::Human)
        );
        assert!(recovered.submit().is_err());
        assert!(recovered.reserved().is_err());
        recovered.status(Some(&PaymentStatus::Settled)).unwrap();
        assert!(Action::recover(
            TaskKind::L402Resource,
            &("other", "invoice", 3),
            Some(&authority)
        )
        .is_err());
    }
    #[test]
    fn cancellation_and_bounded_trace() {
        let mut a = prepared();
        a.cancel().unwrap();
        assert!(a.human().is_err());
        assert!(a.reserved().is_err());
        let mut a = Action::recover(TaskKind::L402Resource, &"legacy", None).unwrap();
        for _ in 0..100 {
            a.status(None).unwrap();
            a.status(Some(&PaymentStatus::Pending)).unwrap();
        }
        assert!(a.snapshot().trace.len() <= 32);
        assert_eq!(a.snapshot().observed_submission_attempts, 0);
        assert!(a.snapshot().submission_may_have_occurred);
    }
    #[test]
    fn rgb_contract_binds_asset_recipient_invoice_amount_and_carrier() {
        use rgb402_core::{wallet::PaymentRequest, AssetId, PaymentId};
        let original = PaymentRequest {
            asset_id: AssetId::new("rgb:one").unwrap(),
            amount: 5,
            invoice: "invoice".into(),
            payment_hash: PaymentId::new("recipient-hash").unwrap(),
            expires_at: 999,
            network: "Regtest".into(),
            carrier_msat: 3000000,
        };
        let mut action = Action::new(TaskKind::RgbPayment, &original).unwrap();
        action.decoded().unwrap();
        action.validated().unwrap();
        action.policy(false, false).unwrap();
        action.human().unwrap();
        assert_eq!(action.contract().completion, Completion::NodeSettlement);
        for field in 0..7 {
            let mut changed = original.clone();
            match field {
                0 => changed.asset_id = AssetId::new("rgb:other").unwrap(),
                1 => changed.amount = 6,
                2 => changed.invoice = "other-invoice".into(),
                3 => changed.payment_hash = PaymentId::new("other-recipient").unwrap(),
                4 => changed.expires_at = 1000,
                5 => changed.network = "Bitcoin".into(),
                _ => changed.carrier_msat = 4000000,
            };
            assert!(action.authority(&changed).is_err());
        }
    }
}
