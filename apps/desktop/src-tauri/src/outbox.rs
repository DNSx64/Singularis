use std::{
    collections::HashSet,
    env,
    net::IpAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use reqwest::{StatusCode, Url, redirect::Policy};
use serde::Serialize;
use singularis_protocol::{AcceptedEvent, SubmitEvent};
use singularis_vault::{VaultError, VaultOutboxItem};
use thiserror::Error;
use tokio::sync::{Mutex as AsyncMutex, Notify, watch};

use crate::ManagedVault;

const DEFAULT_API_URL: &str = "http://127.0.0.1:8787";
const MAX_RECEIPT_BYTES: usize = 64 * 1024;
const RETRY_INTERVAL: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutboxWorkerState {
    Paused,
    Idle,
    Sending,
    Deferred,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct OutboxWorkerStatus {
    pub state: OutboxWorkerState,
    pub pending: usize,
    pub attempted: usize,
    pub acknowledged: usize,
    pub failed: usize,
    pub last_error: Option<&'static str>,
}

impl OutboxWorkerStatus {
    const fn paused(pending: usize) -> Self {
        Self {
            state: OutboxWorkerState::Paused,
            pending,
            attempted: 0,
            acknowledged: 0,
            failed: 0,
            last_error: None,
        }
    }

    const fn sending(pending: usize) -> Self {
        Self {
            state: OutboxWorkerState::Sending,
            pending,
            attempted: 0,
            acknowledged: 0,
            failed: 0,
            last_error: None,
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum OutboxWorkerConfigError {
    #[error("SINGULARIS_API_URL is not a valid base URL")]
    InvalidUrl,
    #[error("unencrypted relay HTTP is permitted only on loopback")]
    InsecureUrl,
    #[error("the relay HTTP client could not be created")]
    HttpClient(#[source] reqwest::Error),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelaySubmitError {
    Unreachable,
    Rejected,
    InvalidResponse,
}

impl RelaySubmitError {
    const fn code(self) -> &'static str {
        match self {
            Self::Unreachable => "relay_unreachable",
            Self::Rejected => "relay_rejected",
            Self::InvalidResponse => "invalid_relay_response",
        }
    }
}

#[async_trait]
trait RelaySubmitter: Send + Sync {
    async fn submit(&self, payload: Vec<u8>) -> Result<AcceptedEvent, RelaySubmitError>;
}

struct HttpRelaySubmitter {
    client: reqwest::Client,
    submit_url: Url,
}

impl HttpRelaySubmitter {
    fn new(base_url: &str) -> Result<Self, OutboxWorkerConfigError> {
        let submit_url = relay_submit_url(base_url)?;
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(Policy::none())
            .user_agent("singularis-desktop/0.1")
            .build()
            .map_err(OutboxWorkerConfigError::HttpClient)?;
        Ok(Self { client, submit_url })
    }
}

#[async_trait]
impl RelaySubmitter for HttpRelaySubmitter {
    async fn submit(&self, payload: Vec<u8>) -> Result<AcceptedEvent, RelaySubmitError> {
        let mut response = self
            .client
            .post(self.submit_url.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload)
            .send()
            .await
            .map_err(|_| RelaySubmitError::Unreachable)?;
        if response.status() != StatusCode::ACCEPTED {
            return Err(RelaySubmitError::Rejected);
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RECEIPT_BYTES as u64)
        {
            return Err(RelaySubmitError::InvalidResponse);
        }
        let mut body = Vec::with_capacity(response.content_length().unwrap_or(0) as usize);
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| RelaySubmitError::InvalidResponse)?
        {
            if body
                .len()
                .checked_add(chunk.len())
                .is_none_or(|length| length > MAX_RECEIPT_BYTES)
            {
                return Err(RelaySubmitError::InvalidResponse);
            }
            body.extend_from_slice(&chunk);
        }
        if body.is_empty() {
            return Err(RelaySubmitError::InvalidResponse);
        }
        serde_json::from_slice(&body).map_err(|_| RelaySubmitError::InvalidResponse)
    }
}

pub(crate) struct OutboxWorker {
    relay: Arc<dyn RelaySubmitter>,
    enabled: AtomicBool,
    enabled_tx: watch::Sender<bool>,
    kick: Notify,
    run_lock: AsyncMutex<()>,
    status: Mutex<OutboxWorkerStatus>,
}

impl OutboxWorker {
    pub(crate) fn start(vault: ManagedVault) -> Result<Arc<Self>, OutboxWorkerConfigError> {
        let base_url = match env::var("SINGULARIS_API_URL") {
            Ok(base_url) => base_url,
            Err(env::VarError::NotPresent) => DEFAULT_API_URL.to_owned(),
            Err(env::VarError::NotUnicode(_)) => return Err(OutboxWorkerConfigError::InvalidUrl),
        };
        let relay = Arc::new(HttpRelaySubmitter::new(&base_url)?);
        let worker = Arc::new(Self::new(relay));
        Self::spawn(Arc::clone(&worker), vault);
        Ok(worker)
    }

    fn new(relay: Arc<dyn RelaySubmitter>) -> Self {
        let (enabled_tx, _) = watch::channel(false);
        Self {
            relay,
            enabled: AtomicBool::new(false),
            enabled_tx,
            kick: Notify::new(),
            run_lock: AsyncMutex::new(()),
            status: Mutex::new(OutboxWorkerStatus::paused(0)),
        }
    }

    fn spawn(worker: Arc<Self>, vault: ManagedVault) {
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    () = worker.kick.notified() => {}
                    () = tokio::time::sleep(RETRY_INTERVAL) => {}
                }
                if worker.is_enabled() {
                    worker.flush_now(&vault).await;
                }
            }
        });
    }

    pub(crate) fn resume(&self) {
        if !self.enabled.swap(true, Ordering::AcqRel) {
            self.enabled_tx.send_replace(true);
        }
        self.kick.notify_one();
    }

    pub(crate) fn notify_pending(&self) {
        self.kick.notify_one();
    }

    pub(crate) fn pause(&self) {
        if self.enabled.swap(false, Ordering::AcqRel) {
            self.enabled_tx.send_replace(false);
        }
        let pending = self.status().pending;
        self.set_status(OutboxWorkerStatus::paused(pending));
    }

    pub(crate) async fn pause_and_wait(&self) {
        self.pause();
        let _run_guard = self.run_lock.lock().await;
    }

    pub(crate) fn status(&self) -> OutboxWorkerStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| OutboxWorkerStatus {
                state: OutboxWorkerState::Deferred,
                pending: 0,
                attempted: 0,
                acknowledged: 0,
                failed: 1,
                last_error: Some("worker_state_unavailable"),
            })
    }

    pub(crate) async fn flush_now(&self, vault: &ManagedVault) -> OutboxWorkerStatus {
        let _run_guard = self.run_lock.lock().await;
        if !self.is_enabled() {
            let status = OutboxWorkerStatus::paused(self.status().pending);
            self.set_status(status.clone());
            return status;
        }

        let items = match list_pending(vault) {
            Ok(items) => items,
            Err(VaultError::Locked) => {
                let status = OutboxWorkerStatus::paused(0);
                self.set_status(status.clone());
                return status;
            }
            Err(_) => {
                let status = deferred_status(0, 0, 0, 1, "vault_read_failed");
                self.set_status(status.clone());
                return status;
            }
        };
        self.set_status(OutboxWorkerStatus::sending(items.len()));
        let mut enabled = self.enabled_tx.subscribe();
        let mut report = OutboxWorkerStatus::sending(items.len());
        let mut blocked_devices = HashSet::new();

        for item in items {
            if blocked_devices.contains(&item.device_id) {
                continue;
            }
            if !self.is_enabled() {
                report.state = OutboxWorkerState::Paused;
                break;
            }
            let (submission, payload) = match validate_outbox_item(item) {
                Ok(validated) => validated,
                Err((code, device_id)) => {
                    report.failed += 1;
                    report.last_error.get_or_insert(code);
                    blocked_devices.insert(device_id);
                    continue;
                }
            };
            report.attempted += 1;
            let relay_result = tokio::select! {
                result = self.relay.submit(payload) => result,
                changed = enabled.changed() => {
                    if changed.is_ok() && !*enabled.borrow() {
                        report.state = OutboxWorkerState::Paused;
                        break;
                    }
                    report.state = OutboxWorkerState::Paused;
                    break;
                }
            };
            let receipt = match relay_result {
                Ok(receipt) if valid_receipt(&submission, &receipt) => receipt,
                Ok(_) => {
                    report.failed += 1;
                    report.last_error.get_or_insert("invalid_relay_receipt");
                    blocked_devices.insert(submission.sender_device_id.to_string());
                    continue;
                }
                Err(error) => {
                    report.failed += 1;
                    report.last_error.get_or_insert(error.code());
                    blocked_devices.insert(submission.sender_device_id.to_string());
                    continue;
                }
            };
            match acknowledge(
                vault,
                &submission.sender_device_id.to_string(),
                &receipt.event_id.as_uuid().to_string(),
            ) {
                Ok(true) => report.acknowledged += 1,
                Ok(false) => {}
                Err(VaultError::Locked) => {
                    report.state = OutboxWorkerState::Paused;
                    break;
                }
                Err(_) => {
                    report.failed += 1;
                    report
                        .last_error
                        .get_or_insert("vault_acknowledgement_failed");
                    blocked_devices.insert(submission.sender_device_id.to_string());
                }
            }
        }

        report.pending = list_pending(vault)
            .map(|items| items.len())
            .unwrap_or_else(|_| report.pending.saturating_sub(report.acknowledged));
        if !self.is_enabled() {
            report.state = OutboxWorkerState::Paused;
        } else if report.failed > 0 || report.pending > 0 {
            report.state = OutboxWorkerState::Deferred;
        } else {
            report.state = OutboxWorkerState::Idle;
        }
        self.set_status(report.clone());
        report
    }

    fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    fn set_status(&self, status: OutboxWorkerStatus) {
        if let Ok(mut current) = self.status.lock() {
            *current = status;
        }
    }
}

fn validate_outbox_item(
    item: VaultOutboxItem,
) -> Result<(SubmitEvent, Vec<u8>), (&'static str, String)> {
    let submission: SubmitEvent = serde_json::from_slice(&item.payload)
        .map_err(|_| ("invalid_outbox_entry", item.device_id.clone()))?;
    let canonical = serde_json::to_vec(&submission)
        .map_err(|_| ("invalid_outbox_entry", item.device_id.clone()))?;
    if canonical != item.payload
        || submission.event_id.as_uuid().to_string() != item.event_id
        || submission.sender_device_id.to_string() != item.device_id
    {
        return Err(("invalid_outbox_entry", item.device_id));
    }
    Ok((submission, canonical))
}

fn valid_receipt(submission: &SubmitEvent, receipt: &AcceptedEvent) -> bool {
    receipt.event_id == submission.event_id
        && receipt.channel_id == submission.channel_id
        && receipt.sequence > 0
        && receipt.expires_at > receipt.accepted_at
}

fn list_pending(vault: &ManagedVault) -> Result<Vec<VaultOutboxItem>, VaultError> {
    vault
        .lock()
        .map_err(|_| VaultError::CorruptState)?
        .list_all_mls_outbox()
}

fn acknowledge(vault: &ManagedVault, device_id: &str, event_id: &str) -> Result<bool, VaultError> {
    vault
        .lock()
        .map_err(|_| VaultError::CorruptState)?
        .acknowledge_mls_outbox(device_id, event_id)
}

const fn deferred_status(
    pending: usize,
    attempted: usize,
    acknowledged: usize,
    failed: usize,
    error: &'static str,
) -> OutboxWorkerStatus {
    OutboxWorkerStatus {
        state: OutboxWorkerState::Deferred,
        pending,
        attempted,
        acknowledged,
        failed,
        last_error: Some(error),
    }
}

fn relay_submit_url(base_url: &str) -> Result<Url, OutboxWorkerConfigError> {
    let mut url = Url::parse(base_url).map_err(|_| OutboxWorkerConfigError::InvalidUrl)?;
    if url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(OutboxWorkerConfigError::InvalidUrl);
    }
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_host(&url) => {}
        "http" => return Err(OutboxWorkerConfigError::InsecureUrl),
        _ => return Err(OutboxWorkerConfigError::InvalidUrl),
    }
    url.set_path("/v1/events");
    Ok(url)
}

fn is_loopback_host(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use singularis_protocol::{EventId, OpaqueMlsMessage, ServerTtl};
    use singularis_server::{AppConfig, InMemoryEventStore, router};
    use singularis_vault::{NewVaultMessage, Vault};
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    use super::*;

    const PASSPHRASE: &str = "correct horse battery staple";

    fn submission() -> SubmitEvent {
        SubmitEvent {
            event_id: EventId::new(),
            community_id: Uuid::new_v4(),
            channel_id: Uuid::new_v4(),
            sender_device_id: Uuid::new_v4(),
            sender_counter: 1,
            mls_message: OpaqueMlsMessage::from_bytes(b"opaque worker MLS message").unwrap(),
            ttl_seconds: ServerTtl::MIN,
        }
    }

    fn queued_vault(submission: &SubmitEvent) -> (tempfile::TempDir, ManagedVault) {
        let temporary_directory = tempdir().unwrap();
        let mut vault = Vault::new(temporary_directory.path().join("vault-v1"));
        vault.initialize(PASSPHRASE).unwrap();
        let payload = serde_json::to_vec(submission).unwrap();
        vault
            .store_mls_snapshot_and_outbox(
                &submission.sender_device_id.to_string(),
                b"worker test snapshot",
                &submission.event_id.as_uuid().to_string(),
                &payload,
            )
            .unwrap();
        (temporary_directory, Arc::new(Mutex::new(vault)))
    }

    #[tokio::test]
    async fn http_worker_flushes_and_acknowledges_a_pending_event() {
        let submission = submission();
        let (_temporary_directory, vault) = queued_vault(&submission);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = router(
            AppConfig::default(),
            Arc::new(InMemoryEventStore::default()),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let relay = Arc::new(HttpRelaySubmitter::new(&format!("http://{address}")).unwrap());
        let worker = OutboxWorker::new(relay);
        worker.resume();

        let report = worker.flush_now(&vault).await;

        assert_eq!(report.state, OutboxWorkerState::Idle);
        assert_eq!(report.attempted, 1);
        assert_eq!(report.acknowledged, 1);
        assert_eq!(report.pending, 0);
        assert!(
            vault
                .lock()
                .unwrap()
                .list_all_mls_outbox()
                .unwrap()
                .is_empty()
        );
        server.abort();
    }

    #[tokio::test]
    async fn native_message_queue_is_relayed_and_acknowledged() {
        let temporary_directory = tempdir().unwrap();
        let mut unlocked_vault = Vault::new(temporary_directory.path().join("vault-v1"));
        unlocked_vault.initialize(PASSPHRASE).unwrap();
        let message = NewVaultMessage {
            id: Uuid::new_v4().to_string(),
            channel_id: "briefing".to_owned(),
            body: "native service to relay canary".to_owned(),
            created_at_ms: 1_754_700_000_000,
        };
        crate::messaging::queue_message(&unlocked_vault, &message).unwrap();
        let vault = Arc::new(Mutex::new(unlocked_vault));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = router(
            AppConfig::default(),
            Arc::new(InMemoryEventStore::default()),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let relay = Arc::new(HttpRelaySubmitter::new(&format!("http://{address}")).unwrap());
        let worker = OutboxWorker::new(relay);
        worker.resume();

        let report = worker.flush_now(&vault).await;

        assert_eq!(report.state, OutboxWorkerState::Idle);
        assert_eq!(report.acknowledged, 1);
        assert_eq!(report.pending, 0);
        let vault = vault.lock().unwrap();
        assert!(vault.list_all_mls_outbox().unwrap().is_empty());
        assert_eq!(
            vault.list_messages("briefing").unwrap()[0].body,
            message.body
        );
        server.abort();
    }

    #[tokio::test]
    async fn http_submitter_rejects_an_oversized_chunked_receipt() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let mut buffer = [0_u8; 1024];
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&buffer[..read]);
            }
            stream
                .write_all(
                    b"HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            let oversized_body = vec![b'x'; MAX_RECEIPT_BYTES + 1];
            for chunk in oversized_body.chunks(16 * 1024) {
                stream
                    .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                    .await
                    .unwrap();
                stream.write_all(chunk).await.unwrap();
                stream.write_all(b"\r\n").await.unwrap();
            }
            stream.write_all(b"0\r\n\r\n").await.unwrap();
        });
        let relay = HttpRelaySubmitter::new(&format!("http://{address}")).unwrap();

        let result = relay.submit(b"{}".to_vec()).await;

        assert_eq!(result, Err(RelaySubmitError::InvalidResponse));
        server.await.unwrap();
    }

    struct LockCheckingRelay {
        vault: ManagedVault,
        calls: AtomicUsize,
        receipt: AcceptedEvent,
    }

    struct BlockingRelay {
        started: Notify,
        release: Notify,
        cancelled: AtomicBool,
    }

    struct InFlightRequest<'a>(&'a AtomicBool);

    impl Drop for InFlightRequest<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl RelaySubmitter for BlockingRelay {
        async fn submit(&self, _payload: Vec<u8>) -> Result<AcceptedEvent, RelaySubmitError> {
            let _in_flight = InFlightRequest(&self.cancelled);
            self.started.notify_one();
            self.release.notified().await;
            Err(RelaySubmitError::Unreachable)
        }
    }

    #[async_trait]
    impl RelaySubmitter for LockCheckingRelay {
        async fn submit(&self, _payload: Vec<u8>) -> Result<AcceptedEvent, RelaySubmitError> {
            assert!(
                self.vault.try_lock().is_ok(),
                "vault lock leaked into network I/O"
            );
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.receipt.clone())
        }
    }

    #[tokio::test]
    async fn worker_releases_the_vault_lock_before_network_io() {
        let submission = submission();
        let (_temporary_directory, vault) = queued_vault(&submission);
        let receipt = AcceptedEvent {
            event_id: submission.event_id,
            channel_id: submission.channel_id,
            sequence: 1,
            accepted_at: time::OffsetDateTime::UNIX_EPOCH,
            expires_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(5),
        };
        let relay = Arc::new(LockCheckingRelay {
            vault: Arc::clone(&vault),
            calls: AtomicUsize::new(0),
            receipt,
        });
        let worker = OutboxWorker::new(relay.clone());
        worker.resume();

        let report = worker.flush_now(&vault).await;

        assert_eq!(relay.calls.load(Ordering::SeqCst), 1);
        assert_eq!(report.acknowledged, 1);
    }

    #[tokio::test]
    async fn pausing_cancels_an_in_flight_request_and_preserves_the_entry() {
        let submission = submission();
        let (_temporary_directory, vault) = queued_vault(&submission);
        let relay = Arc::new(BlockingRelay {
            started: Notify::new(),
            release: Notify::new(),
            cancelled: AtomicBool::new(false),
        });
        let worker = Arc::new(OutboxWorker::new(relay.clone()));
        worker.resume();
        let flush_worker = Arc::clone(&worker);
        let flush_vault = Arc::clone(&vault);
        let flush = tokio::spawn(async move { flush_worker.flush_now(&flush_vault).await });
        relay.started.notified().await;

        tokio::time::timeout(Duration::from_secs(1), worker.pause_and_wait())
            .await
            .expect("pause must wait for request cancellation");
        assert!(relay.cancelled.load(Ordering::SeqCst));
        let report = tokio::time::timeout(Duration::from_secs(1), flush)
            .await
            .expect("cancelled flush must finish")
            .unwrap();

        assert_eq!(report.state, OutboxWorkerState::Paused);
        assert_eq!(report.acknowledged, 0);
        assert_eq!(
            vault.lock().unwrap().list_all_mls_outbox().unwrap().len(),
            1
        );
    }

    #[test]
    fn relay_url_rejects_insecure_remote_http_and_ambiguous_paths() {
        assert!(matches!(
            relay_submit_url("http://example.com"),
            Err(OutboxWorkerConfigError::InsecureUrl)
        ));
        assert!(matches!(
            relay_submit_url("https://example.com/unexpected"),
            Err(OutboxWorkerConfigError::InvalidUrl)
        ));
        assert_eq!(
            relay_submit_url("http://127.0.0.1:8787").unwrap().as_str(),
            "http://127.0.0.1:8787/v1/events"
        );
    }
}
