//! Human BTC transfers: mandatory approval, immutable recipient binding, durable reservation.
use crate::{
    harness::{Action, ReservationAuthority, TaskKind, TaskSnapshot},
    lightning::{ApprovedLightningPayment, LightningInvoice, LightningNode},
    wallet::WalletError,
};
use rgb402_core::{
    btc::BtcTransferPolicy, wallet::PolicyDecision, PaymentId, PaymentStatus, UnixTimestamp,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BtcRecipient {
    pub identifier: String,
    pub authoritative_domain: String,
    pub service_url: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Contract {
    recipient: BtcRecipient,
    invoice: LightningInvoice,
}
#[derive(Clone, Serialize, Deserialize)]
struct Reservation {
    contract: Contract,
    at: u64,
    authority: ReservationAuthority,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BtcPlanView {
    pub plan_id: String,
    pub recipient: BtcRecipient,
    pub amount_sats: String,
    pub available_sats: String,
    pub payment_hash: PaymentId,
    pub expires_at: u64,
    pub policy: PolicyDecision,
}
struct Plan {
    contract: Contract,
    action: Action,
}
pub struct BtcService {
    node: Arc<dyn LightningNode>,
    policy: BtcTransferPolicy,
    journal: File,
    lock: PathBuf,
    poisoned: bool,
    reservations: Vec<Reservation>,
    plans: HashMap<String, Plan>,
    actions: HashMap<PaymentId, Action>,
    sequence: u64,
}
impl Drop for BtcService {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock);
    }
}
impl BtcService {
    pub fn policy(&self) -> &BtcTransferPolicy {
        &self.policy
    }
    pub async fn balance(&self) -> Result<u64, WalletError> {
        self.node.outbound_sats().await
    }
    pub fn from_env(node: Arc<dyn LightningNode>, wallet_path: &Path) -> Result<Self, WalletError> {
        let number = |key: &str, default: u64| -> Result<u64, WalletError> {
            match std::env::var(key) {
                Ok(s) => s
                    .parse()
                    .map_err(|_| WalletError::Config(format!("invalid {key}"))),
                Err(std::env::VarError::NotPresent) => Ok(default),
                _ => Err(WalletError::Config(format!("invalid {key}"))),
            }
        };
        let path = PathBuf::from(format!("{}.btc.jsonl", wallet_path.display()));
        Self::open(
            node,
            BtcTransferPolicy {
                max_payment_sats: number("MAX_BTC_TRANSFER_SATS", 100)?,
                max_daily_sats: number("MAX_BTC_TRANSFER_DAILY_SATS", 500)?,
            },
            &path,
        )
    }
    pub fn open(
        node: Arc<dyn LightningNode>,
        policy: BtcTransferPolicy,
        path: &Path,
    ) -> Result<Self, WalletError> {
        if policy.max_payment_sats == 0 || policy.max_payment_sats > policy.max_daily_sats {
            return Err(WalletError::Config(
                "require 0 < BTC transfer limit <= daily limit".into(),
            ));
        }
        let lock = path.with_extension("lock");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&lock)?
            .sync_all()?;
        let loaded = (|| -> Result<_, WalletError> {
            let journal = OpenOptions::new()
                .read(true)
                .append(true)
                .create(true)
                .mode(0o600)
                .open(path)?;
            let mut reservations = Vec::new();
            let mut actions = HashMap::new();
            for line in BufReader::new(&journal).lines() {
                let r: Reservation = serde_json::from_str(&line?)?;
                let action =
                    Action::recover(TaskKind::BtcTransfer, &r.contract, Some(&r.authority))?;
                if actions
                    .insert(r.contract.invoice.payment_hash.clone(), action)
                    .is_some()
                {
                    return Err(WalletError::Invalid("duplicate BTC journal entry"));
                }
                reservations.push(r);
            }
            File::open(
                path.parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
            )?
            .sync_all()?;
            Ok((journal, reservations, actions))
        })();
        let (journal, reservations, actions) = match loaded {
            Ok(v) => v,
            Err(e) => {
                let _ = std::fs::remove_file(&lock);
                return Err(e);
            }
        };
        Ok(Self {
            node,
            policy,
            journal,
            lock,
            poisoned: false,
            reservations,
            actions,
            plans: HashMap::new(),
            sequence: 0,
        })
    }
    fn spent(&self, now: u64) -> u64 {
        self.reservations
            .iter()
            .filter(|r| r.at / 86400 >= now / 86400)
            .fold(0u64, |n, r| {
                n.saturating_add(r.contract.invoice.amount_sats)
            })
    }
    pub async fn prepare(
        &mut self,
        recipient: BtcRecipient,
        invoice: LightningInvoice,
    ) -> Result<BtcPlanView, WalletError> {
        if self.actions.contains_key(&invoice.payment_hash) {
            return Err(WalletError::Duplicate);
        }
        let now = UnixTimestamp::now().seconds();
        if invoice.expires_at <= now
            || self.node.decode_btc_invoice(&invoice.invoice).await? != invoice
        {
            return Err(WalletError::Invalid("BTC invoice validation mismatch"));
        }
        let available = self.node.outbound_sats_for(invoice.amount_sats).await?;
        let policy = self
            .policy
            .evaluate(invoice.amount_sats, available, self.spent(now));
        let contract = Contract {
            recipient: recipient.clone(),
            invoice: invoice.clone(),
        };
        let mut action = Action::new(TaskKind::BtcTransfer, &contract)?;
        action.decoded()?;
        action.validated()?;
        action.policy(false, matches!(policy, PolicyDecision::Deny { .. }))?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(WalletError::Invalid("plan counter overflow"))?;
        let id = format!(
            "btc-{}-{}",
            action.contract().economic_action_id,
            self.sequence
        );
        let view = BtcPlanView {
            plan_id: id.clone(),
            recipient,
            amount_sats: invoice.amount_sats.to_string(),
            available_sats: available.to_string(),
            payment_hash: invoice.payment_hash,
            expires_at: invoice.expires_at,
            policy,
        };
        self.plans.insert(id, Plan { contract, action });
        Ok(view)
    }
    pub fn confirm_from_human(&mut self, id: &str, yes: bool) -> Result<(), WalletError> {
        let plan = self.plans.get_mut(id).ok_or(WalletError::UnknownPlan)?;
        if yes {
            plan.action.human()
        } else {
            plan.action.cancel()
        }
    }
    pub fn plan_hash(&self, id: &str) -> Result<PaymentId, WalletError> {
        Ok(self
            .plans
            .get(id)
            .ok_or(WalletError::UnknownPlan)?
            .contract
            .invoice
            .payment_hash
            .clone())
    }
    pub fn plan_task(&self, id: &str) -> Option<TaskSnapshot> {
        self.plans.get(id).map(|p| p.action.snapshot())
    }
    pub fn task(&self, hash: &PaymentId) -> Option<TaskSnapshot> {
        self.actions.get(hash).map(Action::snapshot)
    }
    pub fn history(&self) -> Vec<(PaymentId, u64, u64)> {
        self.reservations
            .iter()
            .map(|r| {
                (
                    r.contract.invoice.payment_hash.clone(),
                    r.contract.invoice.amount_sats,
                    r.at,
                )
            })
            .collect()
    }
    pub async fn status(&mut self, hash: &PaymentId) -> Result<PaymentStatus, WalletError> {
        let result = self.node.btc_payment(hash).await.map(|p| p.status);
        if let Some(action) = self.actions.get_mut(hash) {
            action.status(result.as_ref().ok())?;
        }
        result
    }
    pub async fn execute(&mut self, id: &str) -> Result<PaymentId, WalletError> {
        if self.poisoned {
            return Err(WalletError::Invalid(
                "BTC journal write failed; inspect state",
            ));
        }
        let plan = self.plans.get(id).ok_or(WalletError::UnknownPlan)?;
        let contract = plan.contract.clone();
        let hash = contract.invoice.payment_hash.clone();
        if self.actions.contains_key(&hash) {
            return Err(WalletError::Duplicate);
        }
        let authority = plan.action.authority(&contract)?;
        let now = UnixTimestamp::now().seconds();
        if contract.invoice.expires_at <= now
            || self
                .node
                .decode_btc_invoice(&contract.invoice.invoice)
                .await?
                != contract.invoice
        {
            return Err(WalletError::Invalid("BTC invoice changed or expired"));
        }
        if let PolicyDecision::Deny { reason } = self.policy.evaluate(
            contract.invoice.amount_sats,
            self.node
                .outbound_sats_for(contract.invoice.amount_sats)
                .await?,
            self.spent(now),
        ) {
            return Err(WalletError::Denied(reason));
        }
        let reservation = Reservation {
            contract: contract.clone(),
            at: now,
            authority,
        };
        let mut bytes = serde_json::to_vec(&reservation)?;
        bytes.push(b'\n');
        self.poisoned = true;
        self.journal.write_all(&bytes)?;
        self.journal.sync_all()?;
        self.reservations.push(reservation);
        let mut plan = self.plans.remove(id).ok_or(WalletError::UnknownPlan)?;
        plan.action.reserved()?;
        plan.action.submit()?;
        self.actions.insert(hash.clone(), plan.action);
        self.poisoned = false;
        // A transport error never authorizes another send. Recover exclusively via status.
        let _result = self
            .node
            .send_btc(&ApprovedLightningPayment(contract.invoice))
            .await;
        Ok(hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lightning::LightningPayment, wallet::PaymentResult};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    struct Node {
        sends: AtomicU64,
        amount: AtomicU64,
        balance: AtomicU64,
        minimum: AtomicU64,
        fail: AtomicBool,
    }
    fn invoice(n: u64, amount: u64) -> LightningInvoice {
        LightningInvoice {
            invoice: n.to_string(),
            payment_hash: PaymentId::new(format!("{n:064x}")).unwrap(),
            amount_sats: amount,
            expires_at: u64::MAX,
        }
    }
    #[async_trait]
    impl LightningNode for Node {
        async fn create_invoice(&self, _: u64, _: u32) -> Result<LightningInvoice, WalletError> {
            panic!("sender must not issue invoices")
        }
        async fn decode_btc_invoice(&self, s: &str) -> Result<LightningInvoice, WalletError> {
            Ok(invoice(
                s.parse().unwrap(),
                self.amount.load(Ordering::SeqCst),
            ))
        }
        async fn outbound_sats(&self) -> Result<u64, WalletError> {
            Ok(self.balance.load(Ordering::SeqCst))
        }
        async fn outbound_sats_for(&self, amount: u64) -> Result<u64, WalletError> {
            if amount < self.minimum.load(Ordering::SeqCst) {
                Ok(0)
            } else {
                self.outbound_sats().await
            }
        }
        async fn send_btc(
            &self,
            p: &ApprovedLightningPayment,
        ) -> Result<PaymentResult, WalletError> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                return Err(WalletError::Node("unknown submission"));
            }
            Ok(PaymentResult {
                payment_id: p.invoice().payment_hash.clone(),
                payment_hash: p.invoice().payment_hash.clone(),
                status: PaymentStatus::Pending,
            })
        }
        async fn btc_payment(&self, _: &PaymentId) -> Result<LightningPayment, WalletError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(WalletError::Node("status unavailable"));
            }
            Ok(LightningPayment::new(PaymentStatus::Settled, None))
        }
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "btc-service-{}-{}.jsonl",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ))
    }
    fn node() -> Arc<Node> {
        Arc::new(Node {
            sends: AtomicU64::new(0),
            amount: AtomicU64::new(5),
            balance: AtomicU64::new(50),
            minimum: AtomicU64::new(1),
            fail: AtomicBool::new(false),
        })
    }
    fn recipient() -> BtcRecipient {
        BtcRecipient {
            identifier: "bob@example.com".into(),
            authoritative_domain: "example.com".into(),
            service_url: "https://example.com/btc/bob".into(),
        }
    }
    fn open(n: Arc<Node>, p: &Path) -> BtcService {
        BtcService::open(
            n,
            BtcTransferPolicy {
                max_payment_sats: 10,
                max_daily_sats: 10,
            },
            p,
        )
        .unwrap()
    }
    #[tokio::test]
    async fn approval_is_required_exact_and_once_with_restart_recovery() {
        let p = path();
        let n = node();
        let mut s = open(n.clone(), &p);
        let plan = s.prepare(recipient(), invoice(1, 5)).await.unwrap();
        assert!(matches!(
            plan.policy,
            PolicyDecision::RequireApproval { .. }
        ));
        assert!(s.execute(&plan.plan_id).await.is_err());
        assert_eq!(n.sends.load(Ordering::SeqCst), 0);
        assert!(s.confirm_from_human("wrong", true).is_err());
        s.confirm_from_human(&plan.plan_id, true).unwrap();
        let hash = s.execute(&plan.plan_id).await.unwrap();
        assert_eq!(n.sends.load(Ordering::SeqCst), 1);
        assert!(s.execute(&plan.plan_id).await.is_err());
        assert_eq!(s.status(&hash).await.unwrap(), PaymentStatus::Settled);
        assert_eq!(s.task(&hash).unwrap().observed_submission_attempts, 1);
        assert!(BtcService::open(
            n.clone(),
            BtcTransferPolicy {
                max_payment_sats: 10,
                max_daily_sats: 10
            },
            &p
        )
        .is_err());
        drop(s);
        let mut s = open(n.clone(), &p);
        assert!(matches!(
            s.prepare(recipient(), invoice(1, 5)).await,
            Err(WalletError::Duplicate)
        ));
        assert_eq!(s.task(&hash).unwrap().observed_submission_attempts, 0);
        s.status(&hash).await.unwrap();
        assert_eq!(n.sends.load(Ordering::SeqCst), 1);
        drop(s);
        std::fs::remove_file(p).unwrap();
    }
    #[tokio::test]
    async fn cancellation_changed_invoice_and_balance_fail_closed() {
        for scenario in 0..3 {
            let p = path();
            let n = node();
            let mut s = open(n.clone(), &p);
            let v = s.prepare(recipient(), invoice(1, 5)).await.unwrap();
            s.confirm_from_human(&v.plan_id, scenario != 0).unwrap();
            if scenario == 1 {
                n.amount.store(6, Ordering::SeqCst)
            }
            if scenario == 2 {
                n.balance.store(0, Ordering::SeqCst)
            }
            assert!(s.execute(&v.plan_id).await.is_err());
            assert_eq!(n.sends.load(Ordering::SeqCst), 0);
            if scenario == 0 {
                assert!(s.confirm_from_human(&v.plan_id, true).is_err())
            }
            drop(s);
            std::fs::remove_file(p).unwrap();
        }
    }
    #[tokio::test]
    async fn uncertain_submission_is_counted_and_never_retried() {
        let p = path();
        let n = node();
        let mut s = open(n.clone(), &p);
        n.fail.store(true, Ordering::SeqCst);
        let a = s.prepare(recipient(), invoice(1, 5)).await.unwrap();
        s.confirm_from_human(&a.plan_id, true).unwrap();
        let h = s.execute(&a.plan_id).await.unwrap();
        assert!(s.status(&h).await.is_err());
        drop(s);
        let mut s = open(n.clone(), &p);
        assert!(s.prepare(recipient(), invoice(1, 5)).await.is_err());
        let b = s.prepare(recipient(), invoice(2, 5)).await.unwrap();
        s.confirm_from_human(&b.plan_id, true).unwrap();
        s.execute(&b.plan_id).await.unwrap();
        let denied = s.prepare(recipient(), invoice(3, 5)).await.unwrap();
        assert!(matches!(denied.policy, PolicyDecision::Deny { .. }));
        assert!(s.confirm_from_human(&denied.plan_id, true).is_err());
        assert_eq!(n.sends.load(Ordering::SeqCst), 2);
        drop(s);
        std::fs::remove_file(p).unwrap();
    }
    #[tokio::test]
    async fn journal_failure_and_cross_plan_approval_cannot_send() {
        let p = path();
        let n = node();
        let mut s = open(n.clone(), &p);
        let a = s.prepare(recipient(), invoice(1, 5)).await.unwrap();
        let b = s.prepare(recipient(), invoice(2, 5)).await.unwrap();
        s.confirm_from_human(&a.plan_id, true).unwrap();
        assert!(s.execute(&b.plan_id).await.is_err());
        s.journal = OpenOptions::new().read(true).open(&p).unwrap();
        assert!(s.execute(&a.plan_id).await.is_err());
        assert!(s.execute(&a.plan_id).await.is_err());
        assert_eq!(n.sends.load(Ordering::SeqCst), 0);
        drop(s);
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn btc_contract_binds_recipient_and_invoice() {
        let c = Contract {
            recipient: recipient(),
            invoice: invoice(1, 5),
        };
        let mut a = Action::new(TaskKind::BtcTransfer, &c).unwrap();
        a.decoded().unwrap();
        a.validated().unwrap();
        a.policy(false, false).unwrap();
        a.human().unwrap();
        for i in 0..4 {
            let mut changed = c.clone();
            match i {
                0 => changed.recipient.identifier = "alice@example.com".into(),
                1 => changed.recipient.service_url = "https://example.com/other".into(),
                2 => changed.invoice.amount_sats = 6,
                _ => changed.invoice.invoice = "other".into(),
            };
            assert!(a.authority(&changed).is_err())
        }
    }
    #[tokio::test]
    async fn channel_minimum_is_checked_before_approval_and_again_before_submission() {
        let p = path();
        let n = node();
        let mut s = open(n.clone(), &p);
        n.minimum.store(3000, Ordering::SeqCst);
        let denied = s.prepare(recipient(), invoice(1, 5)).await.unwrap();
        assert!(matches!(denied.policy, PolicyDecision::Deny { .. }));
        assert!(s.confirm_from_human(&denied.plan_id, true).is_err());
        n.minimum.store(1, Ordering::SeqCst);
        let v = s.prepare(recipient(), invoice(2, 5)).await.unwrap();
        s.confirm_from_human(&v.plan_id, true).unwrap();
        n.minimum.store(3000, Ordering::SeqCst);
        assert!(s.execute(&v.plan_id).await.is_err());
        assert_eq!(n.sends.load(Ordering::SeqCst), 0);
        drop(s);
        std::fs::remove_file(p).unwrap();
    }
}
