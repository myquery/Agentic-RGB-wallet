//! Narrow L402 buyer: one URL, one durable reservation, then proof-only retries.
use crate::harness::{Action, ReservationAuthority, TaskKind, TaskSnapshot};
use crate::{
    lightning::{ApprovedMachinePayment, LightningInvoice, LightningNode},
    wallet::WalletError,
};
use fs2::FileExt;
use reqwest::{header, Client, Url};
use rgb402_core::{
    machine::{MachineDecision, MachinePolicy},
    PaymentId, PaymentStatus,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct L402Challenge {
    pub resource: String,
    pub amount_sats: u64,
    pub invoice: String,
}
// Authentication material is never exposed by public views or Debug.
#[derive(Clone, Serialize, Deserialize)]
struct Purchase {
    url: String,
    invoice: LightningInvoice,
    macaroon: String,
    at: u64,
    auto_approved: bool,
    #[serde(default)]
    authority: Option<ReservationAuthority>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachinePlanView {
    pub plan_id: String,
    pub url: String,
    pub cost_sats: u64,
    pub available_sats: u64,
    pub auto_approve_up_to_sats: u64,
    pub policy: MachineDecision,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PurchaseView {
    pub resource_status: String,
    pub url: String,
    pub cost_sats: u64,
    pub auto_approved: bool,
    pub auto_approve_up_to_sats: u64,
    pub policy: MachineDecision,
    pub payment_hash: Option<PaymentId>,
    pub payment_status: Option<String>,
    pub http_status: Option<u16>,
    pub resource: Option<serde_json::Value>,
    pub plan: Option<MachinePlanView>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachineActivity {
    pub payment_hash: PaymentId,
    pub url: String,
    pub amount_sats: u64,
    pub timestamp: u64,
    pub auto_approved: bool,
    pub status: String,
}
pub struct CommerceConfig {
    pub origins: Vec<String>,
    pub policy: MachinePolicy,
    pub state_path: PathBuf,
}
impl CommerceConfig {
    pub fn from_env() -> Result<Option<Self>, WalletError> {
        let Ok(origins) = std::env::var("ALLOWED_L402_ORIGINS") else {
            return Ok(None);
        };
        let number = |name: &str| -> Result<u64, WalletError> {
            std::env::var(name)
                .ok()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| WalletError::Config(format!("missing or invalid {name}")))
        };
        Ok(Some(Self {
            origins: origins.split(',').map(|s| s.trim().to_owned()).collect(),
            policy: MachinePolicy {
                auto_approve_below_sats: number("AUTO_APPROVE_MACHINE_BELOW_SATS")?,
                max_single_payment_sats: number("MAX_MACHINE_PAYMENT_SATS")?,
                max_daily_spend_sats: number("MAX_MACHINE_DAILY_SPEND_SATS")?,
            },
            state_path: std::env::var("MACHINE_STATE_PATH")
                .unwrap_or_else(|_| ".machine-state.jsonl".into())
                .into(),
        }))
    }
}
pub struct CommerceService {
    node: Arc<dyn LightningNode>,
    http: Client,
    origins: Vec<String>,
    policy: MachinePolicy,
    reservations: Vec<Purchase>,
    plans: HashMap<String, (Purchase, MachinePlanView)>,
    approved: std::collections::HashSet<String>,
    file: File,
    // Held for the service lifetime; process death releases it automatically.
    _lock: File,
    poisoned: bool,
    tasks: std::sync::Mutex<HashMap<String, Action>>,
}
impl CommerceService {
    pub fn open(node: Arc<dyn LightningNode>, config: CommerceConfig) -> Result<Self, WalletError> {
        let mut origins = vec![];
        for origin in config.origins {
            let url = Url::parse(&origin)
                .map_err(|_| WalletError::Config("invalid L402 origin".into()))?;
            // This milestone supports explicitly configured numeric loopback hosts only.
            // No DNS resolution or external/internal network destinations are exposed.
            if url.scheme() != "http"
                || url.host_str() != Some("127.0.0.1")
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(WalletError::Config(
                    "L402 origins must be explicit http://127.0.0.1:port origins".into(),
                ));
            }
            origins.push(url.origin().ascii_serialization());
        }
        if origins.is_empty() {
            return Err(WalletError::Config("L402 origin allowlist is empty".into()));
        }
        let http = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| WalletError::Node("L402 client initialization failed"))?;
        let lock = config.state_path.with_extension("lock");
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&lock)?;
        lock_file.try_lock_exclusive()?;
        lock_file.sync_all()?;
        let opened = (|| -> Result<_, WalletError> {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .read(true)
                .mode(0o600)
                .open(&config.state_path)?;
            let mut reservations = vec![];
            for line in BufReader::new(&file).lines() {
                reservations.push(serde_json::from_str::<Purchase>(&line?)?);
            }
            file.sync_all()?;
            File::open(
                config
                    .state_path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
            )?
            .sync_all()?;
            Ok((file, reservations))
        })();
        let (file, reservations) = opened?;
        let mut tasks = HashMap::new();
        for p in &reservations {
            let recovered = match Action::recover(
                TaskKind::L402Resource,
                &(&p.url, &p.invoice, &p.macaroon),
                p.authority.as_ref(),
            ) {
                Ok(task) => task,
                Err(error) => {
                    return Err(error);
                }
            };
            tasks.insert(p.url.clone(), recovered);
        }
        Ok(Self {
            node,
            http,
            origins,
            policy: config.policy,
            reservations,
            plans: HashMap::new(),
            approved: Default::default(),
            file,
            _lock: lock_file,
            poisoned: false,
            tasks: std::sync::Mutex::new(tasks),
        })
    }
    pub fn task(&self, url: &str) -> Option<TaskSnapshot> {
        self.tasks
            .lock()
            .expect("task lock")
            .get(url)
            .map(Action::snapshot)
    }
    fn update_task(
        &self,
        url: &str,
        f: impl FnOnce(&mut Action) -> Result<(), WalletError>,
    ) -> Result<(), WalletError> {
        f(self
            .tasks
            .lock()
            .expect("task lock")
            .get_mut(url)
            .ok_or(WalletError::Invalid("missing economic task"))?)
    }
    pub fn default_resource(&self) -> String {
        format!("{}/premium/report", self.origins[0])
    }
    fn url(&self, input: &str) -> Result<Url, WalletError> {
        let url = Url::parse(input).map_err(|_| WalletError::Invalid("invalid resource URL"))?;
        if input.len() > 2048
            || url.scheme() != "http"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !self.origins.contains(&url.origin().ascii_serialization())
        {
            return Err(WalletError::Invalid(
                "resource origin or URL is not allowed",
            ));
        }
        Ok(url)
    }
    fn spent(&self) -> Result<u64, WalletError> {
        self.reservations
            .iter()
            .filter(|r| r.at / 86400 >= now() / 86400)
            .try_fold(0u64, |sum, p| {
                sum.checked_add(p.invoice.amount_sats)
                    .ok_or(WalletError::Invalid("machine spend overflow"))
            })
    }
    fn view(&self, p: &Purchase, status: &str) -> PurchaseView {
        PurchaseView {
            resource_status: status.into(),
            url: p.url.clone(),
            cost_sats: p.invoice.amount_sats,
            auto_approved: p.auto_approved,
            auto_approve_up_to_sats: self.policy.auto_approve_below_sats,
            policy: if p.auto_approved {
                MachineDecision::AllowAuto
            } else {
                MachineDecision::RequireApproval
            },
            payment_hash: Some(p.invoice.payment_hash.clone()),
            payment_status: None,
            http_status: None,
            resource: None,
            plan: None,
        }
    }
    async fn response_body(mut response: reqwest::Response) -> Result<Vec<u8>, WalletError> {
        let mut bytes = vec![];
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| WalletError::Node("resource body unavailable"))?
        {
            if bytes.len() + chunk.len() > 65_536 {
                return Err(WalletError::Invalid("resource response too large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
    pub async fn fetch(&mut self, input: &str) -> Result<PurchaseView, WalletError> {
        let url = self.url(input)?;
        let canonical = url.as_str().to_owned();
        tracing::info!(event = "resource_requested");
        // Reservation survives restart and dominates any newly issued challenge.
        if let Some(p) = self
            .reservations
            .iter()
            .find(|p| p.url == canonical)
            .cloned()
        {
            return self.recover(&p).await;
        }
        if let Some((p, plan)) = self
            .plans
            .values()
            .find(|(p, _)| p.url == canonical)
            .cloned()
        {
            if self.approved.remove(&plan.plan_id) {
                return self.execute(p, true).await;
            }
            let mut view = self.view(&p, "approval_required");
            view.plan = Some(plan);
            return Ok(view);
        }
        let response = self
            .http
            .get(url.clone())
            .send()
            .await
            .map_err(|_| WalletError::Node("resource request failed"))?;
        if response.status() == reqwest::StatusCode::OK {
            let body = Self::response_body(response).await?;
            return Ok(PurchaseView {
                resource_status: "free".into(),
                url: canonical,
                cost_sats: 0,
                auto_approved: false,
                auto_approve_up_to_sats: self.policy.auto_approve_below_sats,
                policy: MachineDecision::AllowAuto,
                payment_hash: None,
                payment_status: None,
                http_status: Some(200),
                resource: Some(serde_json::from_slice(&body)?),
                plan: None,
            });
        }
        if response.status() != reqwest::StatusCode::PAYMENT_REQUIRED {
            return Err(WalletError::Http(response.status().as_u16()));
        }
        let auth = response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .and_then(|h| h.to_str().ok())
            .ok_or(WalletError::Invalid("missing L402 challenge"))?;
        let (macaroon, invoice) = parse_challenge(auth)?;
        let body: L402Challenge = serde_json::from_slice(&Self::response_body(response).await?)?;
        if body.resource != url.path() || body.invoice != invoice {
            return Err(WalletError::Invalid(
                "challenge resource or invoice mismatch",
            ));
        }
        let decoded = self.node.decode_btc_invoice(&invoice).await?;
        if decoded.amount_sats != body.amount_sats || decoded.expires_at <= now() {
            return Err(WalletError::Invalid(
                "challenge amount mismatch or expired invoice",
            ));
        }
        if self
            .reservations
            .iter()
            .any(|p| p.invoice.payment_hash == decoded.payment_hash)
        {
            return Err(WalletError::Duplicate);
        }
        tracing::info!(
            event = "payment_challenge_received",
            cost_sats = decoded.amount_sats
        );
        let balance = self.node.outbound_sats().await?;
        let decision = self
            .policy
            .evaluate(decoded.amount_sats, balance, self.spent()?);
        tracing::info!(event="machine_policy_evaluated", decision=?decision, cost_sats=decoded.amount_sats);
        let p = Purchase {
            url: canonical,
            invoice: decoded,
            macaroon,
            at: now(),
            auto_approved: decision == MachineDecision::AllowAuto,
            authority: None,
        };
        let mut action = Action::new(TaskKind::L402Resource, &(&p.url, &p.invoice, &p.macaroon))?;
        action.decoded()?;
        action.validated()?;
        action.policy(
            decision == MachineDecision::AllowAuto,
            matches!(decision, MachineDecision::Deny { .. }),
        )?;
        self.tasks
            .lock()
            .expect("task lock")
            .insert(p.url.clone(), action);
        match &decision {
            MachineDecision::AllowAuto => self.execute(p, false).await,
            MachineDecision::Deny { .. } => {
                let mut v = self.view(&p, "denied");
                v.policy = decision;
                Ok(v)
            }
            MachineDecision::RequireApproval => {
                let plan = MachinePlanView {
                    plan_id: format!(
                        "machine-{}",
                        self.task(&p.url)
                            .ok_or(WalletError::UnknownPlan)?
                            .economic_action_id
                    ),
                    url: p.url.clone(),
                    cost_sats: p.invoice.amount_sats,
                    available_sats: balance,
                    auto_approve_up_to_sats: self.policy.auto_approve_below_sats,
                    policy: decision,
                };
                let mut v = self.view(&p, "approval_required");
                v.plan = Some(plan.clone());
                tracing::info!(event="machine_approval_requested", plan_id=%plan.plan_id);
                self.plans.insert(plan.plan_id.clone(), (p, plan));
                Ok(v)
            }
        }
    }
    /// Trusted application method, not an agent tool. Approval is bound to stored details.
    pub fn confirm_from_human(&mut self, id: &str, yes: bool) -> Result<String, WalletError> {
        let (p, _) = self.plans.get(id).ok_or(WalletError::UnknownPlan)?;
        let url = p.url.clone();
        if yes {
            self.update_task(&url, Action::human)?;
            self.approved.insert(id.into());
        } else {
            self.update_task(&url, Action::cancel)?;
            self.approved.remove(id);
            self.plans.remove(id);
        }
        Ok(url)
    }
    async fn execute(&mut self, mut p: Purchase, human: bool) -> Result<PurchaseView, WalletError> {
        let decoded = self.node.decode_btc_invoice(&p.invoice.invoice).await?;
        if decoded != p.invoice || decoded.expires_at <= now() {
            return Err(WalletError::Invalid(
                "bound machine invoice changed or expired",
            ));
        }
        let decision = self.policy.evaluate(
            decoded.amount_sats,
            self.node.outbound_sats().await?,
            self.spent()?,
        );
        match decision {
            MachineDecision::Deny { reason } => return Err(WalletError::Denied(reason)),
            MachineDecision::RequireApproval if !human => {
                return Err(WalletError::ApprovalRequired)
            }
            _ => (),
        }
        if self.poisoned {
            return Err(WalletError::Invalid(
                "machine journal failed; inspect before restarting",
            ));
        }
        if self
            .reservations
            .iter()
            .any(|r| r.invoice.payment_hash == p.invoice.payment_hash || r.url == p.url)
        {
            return Err(WalletError::Duplicate);
        }
        p.authority = Some(
            self.tasks
                .lock()
                .expect("task lock")
                .get(&p.url)
                .ok_or(WalletError::UnknownPlan)?
                .authority(&(&p.url, &p.invoice, &p.macaroon))?,
        );
        p.at = now();
        self.poisoned = true;
        let mut bytes = serde_json::to_vec(&p)?;
        bytes.push(b'\n');
        self.file.write_all(&bytes)?;
        self.file.sync_all()?;
        self.reservations.push(p.clone());
        self.poisoned = false;
        self.update_task(&p.url, |task| {
            task.reserved()?;
            task.submit()
        })?;
        self.plans.retain(|_, (plan, _)| plan.url != p.url);
        if p.auto_approved {
            tracing::info!(
                event = "machine_payment_auto_approved",
                cost_sats = p.invoice.amount_sats
            );
        }
        tracing::info!(event="machine_payment_submitted",payment_hash=%p.invoice.payment_hash);
        // No automatic resubmission, including node rejection or transport failure.
        let _submission = self
            .node
            .send_btc(&ApprovedMachinePayment(p.invoice.clone()))
            .await;
        self.recover(&p).await
    }
    async fn recover(&self, p: &Purchase) -> Result<PurchaseView, WalletError> {
        let mut view = self.view(p, "payment_unresolved");
        for attempt in 0..100 {
            let payment = match self.node.btc_payment(&p.invoice.payment_hash).await {
                Ok(p) => p,
                Err(_) => {
                    self.update_task(&p.url, |task| task.status(None))?;
                    view.payment_status = Some("uncertain".into());
                    return Ok(view);
                }
            };
            self.update_task(&p.url, |task| task.status(Some(&payment.status)))?;
            match payment.status {
                PaymentStatus::Failed => {
                    view.payment_status = Some("failed".into());
                    tracing::warn!(event = "resource_purchase_failed");
                    return Ok(view);
                }
                PaymentStatus::Pending => {
                    view.payment_status = Some("pending".into());
                    if attempt < 99 {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        continue;
                    }
                    return Ok(view);
                }
                PaymentStatus::Settled => {
                    view.payment_status = Some("settled".into());
                    tracing::info!(event="machine_payment_settled",payment_hash=%p.invoice.payment_hash);
                    let proof = payment.proof(&p.invoice.payment_hash)?;
                    self.update_task(&p.url, Action::proof)?;
                    let mut auth =
                        header::HeaderValue::from_str(&format!("L402 {}:{}", p.macaroon, proof))
                            .map_err(|_| WalletError::Invalid("invalid L402 credential"))?;
                    auth.set_sensitive(true);
                    self.update_task(&p.url, Action::retry_resource)?;
                    tracing::info!(event = "resource_retry_started");
                    view.resource_status = "paid_resource_unavailable".into();
                    let response = match self
                        .http
                        .get(self.url(&p.url)?)
                        .header(header::AUTHORIZATION, auth)
                        .send()
                        .await
                    {
                        Ok(r) => r,
                        Err(_) => return Ok(view),
                    };
                    view.http_status = Some(response.status().as_u16());
                    if response.status() != reqwest::StatusCode::OK {
                        return Ok(view);
                    }
                    let body = match Self::response_body(response).await {
                        Ok(b) => b,
                        Err(_) => return Ok(view),
                    };
                    let resource = match serde_json::from_slice(&body) {
                        Ok(v) => v,
                        Err(_) => return Ok(view),
                    };
                    self.update_task(&p.url, Action::unlocked)?;
                    view.resource = Some(resource);
                    view.resource_status = "purchased".into();
                    tracing::info!(event="resource_unlocked",payment_hash=%p.invoice.payment_hash);
                    return Ok(view);
                }
            }
        }
        Ok(view)
    }
    pub async fn activity(&self) -> Vec<MachineActivity> {
        let mut entries = vec![];
        for p in self.reservations.iter().rev().take(50) {
            let status = match self.node.btc_payment(&p.invoice.payment_hash).await {
                Ok(p) => match p.status {
                    PaymentStatus::Pending => "pending",
                    PaymentStatus::Settled => "settled",
                    PaymentStatus::Failed => "failed",
                },
                Err(_) => "uncertain",
            };
            entries.push(MachineActivity {
                payment_hash: p.invoice.payment_hash.clone(),
                url: p.url.clone(),
                amount_sats: p.invoice.amount_sats,
                timestamp: p.at,
                auto_approved: p.auto_approved,
                status: status.into(),
            });
        }
        entries
    }
}
/// Strict supported L402 profile; reject duplicates, extensions and ambiguous quoting.
fn parse_challenge(value: &str) -> Result<(String, String), WalletError> {
    let rest = value
        .strip_prefix("L402 macaroon=\"")
        .ok_or(WalletError::Invalid("malformed L402 challenge"))?;
    let (macaroon, invoice) = rest
        .split_once("\", invoice=\"")
        .ok_or(WalletError::Invalid("malformed L402 challenge"))?;
    let invoice = invoice
        .strip_suffix('"')
        .ok_or(WalletError::Invalid("malformed L402 challenge"))?;
    if macaroon.is_empty()
        || macaroon.len() > 8192
        || !macaroon
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+/=_-".contains(&b))
        || invoice.len() > 8192
        || !invoice.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(WalletError::Invalid("invalid L402 challenge fields"));
    }
    Ok((macaroon.into(), invoice.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_or_ambiguous_challenges_fail_closed() {
        for value in [
            "",
            "L402",
            "Basic abc",
            "L402 macaroon=\"\", invoice=\"lnbcrt\"",
            "L402 macaroon=\"a\", invoice=\"lnbcrt\", invoice=\"other\"",
            "L402 macaroon=\"a\", invoice=\"lnbcrt\"\r\nInjected: yes",
        ] {
            assert!(parse_challenge(value).is_err());
        }
        assert_eq!(
            parse_challenge("L402 macaroon=\"YWJj==\", invoice=\"lnbcrt123\"").unwrap(),
            ("YWJj==".into(), "lnbcrt123".into())
        );
    }
}
