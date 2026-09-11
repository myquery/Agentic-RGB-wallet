use async_trait::async_trait;
use rgb402_core::{
    PaymentId, PaymentReceipt, PaymentRequest, PaymentRequestId, PaymentStatus, UnixTimestamp,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PaymentError {
    #[error("payment not found: {0}")]
    NotFound(PaymentId),
    #[error("ledger I/O error at {path}: {source}")]
    LedgerIo { path: PathBuf, source: io::Error },
    #[error("ledger encoding error at {path}: {source}")]
    LedgerEncoding {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("payment request conflicts with existing simulated receipt for {payment_id}")]
    ConflictingPaymentRequest { payment_id: PaymentId },
    #[error("real RGB Lightning provider is not implemented in bootstrap; backend={backend:?}")]
    RgbLightningNotImplemented { backend: LightningNodeBackend },
    #[error("invalid deterministic payment id")]
    InvalidPaymentId(#[from] rgb402_core::DomainError),
}

#[async_trait]
pub trait PaymentProvider: Send + Sync {
    async fn pay(&self, request: &PaymentRequest) -> Result<PaymentReceipt, PaymentError>;

    async fn status(&self, payment_id: &PaymentId) -> Result<PaymentStatus, PaymentError>;
}

#[async_trait]
pub trait SettlementVerifier: Send + Sync {
    async fn is_settled(
        &self,
        request: &PaymentRequest,
        payment_id: &PaymentId,
    ) -> Result<bool, PaymentError>;
}

#[derive(Clone)]
pub struct SimulatedRgbPaymentProvider {
    store: Arc<dyn LedgerStore>,
    settlement_time: UnixTimestamp,
}

impl SimulatedRgbPaymentProvider {
    pub fn new() -> Self {
        Self {
            store: Arc::new(InMemoryLedgerStore::default()),
            settlement_time: UnixTimestamp::new(0),
        }
    }

    pub fn with_json_file(path: impl Into<PathBuf>) -> Self {
        Self {
            store: Arc::new(FileLedgerStore::new(path.into())),
            settlement_time: UnixTimestamp::new(0),
        }
    }

    pub fn with_settlement_time(mut self, settlement_time: UnixTimestamp) -> Self {
        self.settlement_time = settlement_time;
        self
    }

    pub fn settled_count(&self) -> Result<usize, PaymentError> {
        self.store.settled_count()
    }

    pub fn deterministic_payment_id(
        payment_request_id: &PaymentRequestId,
    ) -> Result<PaymentId, PaymentError> {
        Ok(PaymentId::new(format!(
            "sim-rgb-payment-{}",
            payment_request_id.as_str()
        ))?)
    }
}

impl Default for SimulatedRgbPaymentProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PaymentProvider for SimulatedRgbPaymentProvider {
    async fn pay(&self, request: &PaymentRequest) -> Result<PaymentReceipt, PaymentError> {
        self.store.settle(request, self.settlement_time)
    }

    async fn status(&self, payment_id: &PaymentId) -> Result<PaymentStatus, PaymentError> {
        self.store.status(payment_id)
    }
}

#[async_trait]
impl SettlementVerifier for SimulatedRgbPaymentProvider {
    async fn is_settled(
        &self,
        request: &PaymentRequest,
        payment_id: &PaymentId,
    ) -> Result<bool, PaymentError> {
        let Some(receipt) = self.store.receipt(payment_id)? else {
            return Ok(false);
        };
        let status = self.store.status(payment_id)?;
        Ok(status == PaymentStatus::Settled
            && receipt.payment_request_id == request.payment_request_id
            && receipt.asset_id == request.asset_id
            && receipt.amount == request.amount)
    }
}

#[derive(Clone, Debug)]
pub struct RgbLightningPaymentProvider {
    config: RgbLightningProviderConfig,
}

impl RgbLightningPaymentProvider {
    pub fn new(config: RgbLightningProviderConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &RgbLightningProviderConfig {
        &self.config
    }
}

#[derive(Clone, Debug)]
pub struct RgbLightningProviderConfig {
    pub backend: LightningNodeBackend,
    pub endpoint: String,
}

impl RgbLightningProviderConfig {
    pub fn polar(endpoint: impl Into<String>) -> Self {
        Self {
            backend: LightningNodeBackend::Polar,
            endpoint: endpoint.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum LightningNodeBackend {
    Polar,
}

#[async_trait]
impl PaymentProvider for RgbLightningPaymentProvider {
    async fn pay(&self, _request: &PaymentRequest) -> Result<PaymentReceipt, PaymentError> {
        Err(PaymentError::RgbLightningNotImplemented {
            backend: self.config.backend.clone(),
        })
    }

    async fn status(&self, _payment_id: &PaymentId) -> Result<PaymentStatus, PaymentError> {
        Err(PaymentError::RgbLightningNotImplemented {
            backend: self.config.backend.clone(),
        })
    }
}

#[async_trait]
impl SettlementVerifier for RgbLightningPaymentProvider {
    async fn is_settled(
        &self,
        _request: &PaymentRequest,
        _payment_id: &PaymentId,
    ) -> Result<bool, PaymentError> {
        Err(PaymentError::RgbLightningNotImplemented {
            backend: self.config.backend.clone(),
        })
    }
}

trait LedgerStore: Send + Sync {
    fn settle(
        &self,
        request: &PaymentRequest,
        settled_at: UnixTimestamp,
    ) -> Result<PaymentReceipt, PaymentError>;
    fn status(&self, payment_id: &PaymentId) -> Result<PaymentStatus, PaymentError>;
    fn receipt(&self, payment_id: &PaymentId) -> Result<Option<PaymentReceipt>, PaymentError>;
    fn settled_count(&self) -> Result<usize, PaymentError>;
}

#[derive(Default)]
struct InMemoryLedgerStore {
    ledger: Mutex<PersistedLedger>,
}

impl LedgerStore for InMemoryLedgerStore {
    fn settle(
        &self,
        request: &PaymentRequest,
        settled_at: UnixTimestamp,
    ) -> Result<PaymentReceipt, PaymentError> {
        let mut ledger = self.ledger.lock().expect("simulated ledger poisoned");
        settle_in_ledger(&mut ledger, request, settled_at)
    }

    fn status(&self, payment_id: &PaymentId) -> Result<PaymentStatus, PaymentError> {
        let ledger = self.ledger.lock().expect("simulated ledger poisoned");
        Ok(ledger
            .records
            .iter()
            .find(|record| &record.receipt.payment_id == payment_id)
            .map(|record| record.status.clone())
            .unwrap_or(PaymentStatus::Failed))
    }

    fn receipt(&self, payment_id: &PaymentId) -> Result<Option<PaymentReceipt>, PaymentError> {
        let ledger = self.ledger.lock().expect("simulated ledger poisoned");
        Ok(ledger
            .records
            .iter()
            .find(|record| &record.receipt.payment_id == payment_id)
            .map(|record| record.receipt.clone()))
    }

    fn settled_count(&self) -> Result<usize, PaymentError> {
        let ledger = self.ledger.lock().expect("simulated ledger poisoned");
        Ok(ledger
            .records
            .iter()
            .filter(|record| record.status == PaymentStatus::Settled)
            .count())
    }
}

struct FileLedgerStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileLedgerStore {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn load(&self) -> Result<PersistedLedger, PaymentError> {
        if !self.path.exists() {
            return Ok(PersistedLedger::default());
        }
        let bytes = fs::read(&self.path).map_err(|source| PaymentError::LedgerIo {
            path: self.path.clone(),
            source,
        })?;
        serde_json::from_slice(&bytes).map_err(|source| PaymentError::LedgerEncoding {
            path: self.path.clone(),
            source,
        })
    }

    fn save(&self, ledger: &PersistedLedger) -> Result<(), PaymentError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|source| PaymentError::LedgerIo {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
        }

        let bytes =
            serde_json::to_vec_pretty(ledger).map_err(|source| PaymentError::LedgerEncoding {
                path: self.path.clone(),
                source,
            })?;
        let tmp_path = temporary_path(&self.path);
        fs::write(&tmp_path, bytes).map_err(|source| PaymentError::LedgerIo {
            path: tmp_path.clone(),
            source,
        })?;
        fs::rename(&tmp_path, &self.path).map_err(|source| PaymentError::LedgerIo {
            path: self.path.clone(),
            source,
        })?;
        Ok(())
    }
}

impl LedgerStore for FileLedgerStore {
    fn settle(
        &self,
        request: &PaymentRequest,
        settled_at: UnixTimestamp,
    ) -> Result<PaymentReceipt, PaymentError> {
        let _guard = self
            .lock
            .lock()
            .expect("simulated ledger file lock poisoned");
        let mut ledger = self.load()?;
        let receipt = settle_in_ledger(&mut ledger, request, settled_at)?;
        self.save(&ledger)?;
        Ok(receipt)
    }

    fn status(&self, payment_id: &PaymentId) -> Result<PaymentStatus, PaymentError> {
        let _guard = self
            .lock
            .lock()
            .expect("simulated ledger file lock poisoned");
        let ledger = self.load()?;
        Ok(ledger
            .records
            .iter()
            .find(|record| &record.receipt.payment_id == payment_id)
            .map(|record| record.status.clone())
            .unwrap_or(PaymentStatus::Failed))
    }

    fn receipt(&self, payment_id: &PaymentId) -> Result<Option<PaymentReceipt>, PaymentError> {
        let _guard = self
            .lock
            .lock()
            .expect("simulated ledger file lock poisoned");
        let ledger = self.load()?;
        Ok(ledger
            .records
            .iter()
            .find(|record| &record.receipt.payment_id == payment_id)
            .map(|record| record.receipt.clone()))
    }

    fn settled_count(&self) -> Result<usize, PaymentError> {
        let _guard = self
            .lock
            .lock()
            .expect("simulated ledger file lock poisoned");
        let ledger = self.load()?;
        Ok(ledger
            .records
            .iter()
            .filter(|record| record.status == PaymentStatus::Settled)
            .count())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PersistedLedger {
    records: Vec<LedgerRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LedgerRecord {
    receipt: PaymentReceipt,
    status: PaymentStatus,
}

fn settle_in_ledger(
    ledger: &mut PersistedLedger,
    request: &PaymentRequest,
    settled_at: UnixTimestamp,
) -> Result<PaymentReceipt, PaymentError> {
    let payment_id =
        SimulatedRgbPaymentProvider::deterministic_payment_id(&request.payment_request_id)?;

    if let Some(record) = ledger
        .records
        .iter()
        .find(|record| record.receipt.payment_id == payment_id)
    {
        if record.receipt.payment_request_id == request.payment_request_id
            && record.receipt.asset_id == request.asset_id
            && record.receipt.amount == request.amount
        {
            return Ok(record.receipt.clone());
        }

        return Err(PaymentError::ConflictingPaymentRequest { payment_id });
    }

    let receipt = PaymentReceipt {
        payment_id,
        payment_request_id: request.payment_request_id.clone(),
        asset_id: request.asset_id.clone(),
        amount: request.amount,
        settled_at,
    };
    ledger.records.push(LedgerRecord {
        receipt: receipt.clone(),
        status: PaymentStatus::Settled,
    });
    Ok(receipt)
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut tmp = path.to_path_buf();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!("{value}.tmp"))
        .unwrap_or_else(|| "tmp".to_owned());
    tmp.set_extension(extension);
    tmp
}

pub mod config;
pub mod rgb;
pub mod wallet;
