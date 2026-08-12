use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use http::{HeaderValue, Method, StatusCode, header::CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use singularis_protocol::{AcceptedEvent, DeliveredEvent, HealthResponse, SubmitEvent};
use time::OffsetDateTime;
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};
use uuid::Uuid;

use crate::{AppConfig, InMemoryEventStore, store::StoreError};

const MAX_EVENT_BODY_BYTES: usize = 384 * 1024;

#[derive(Clone)]
struct AppState {
    config: AppConfig,
    store: Arc<InMemoryEventStore>,
}

pub fn router(config: AppConfig, store: Arc<InMemoryEventStore>) -> Router {
    let state = AppState { config, store };
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list([
            HeaderValue::from_static("http://localhost:1420"),
            HeaderValue::from_static("http://127.0.0.1:1420"),
            HeaderValue::from_static("http://tauri.localhost"),
            HeaderValue::from_static("tauri://localhost"),
        ]))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([CONTENT_TYPE]);

    Router::new()
        .route("/healthz", get(health))
        .route("/v1/events", post(submit_event))
        .route("/v1/channels/{channel_id}/events", get(list_events))
        .layer(DefaultBodyLimit::max(MAX_EVENT_BODY_BYTES))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_owned(),
        service: "singularis-server".to_owned(),
        storage_mode: "volatile-prototype".to_owned(),
        max_server_ttl_seconds: state.config.max_server_ttl.as_seconds(),
    })
}

async fn submit_event(
    State(state): State<AppState>,
    Json(event): Json<SubmitEvent>,
) -> Result<(StatusCode, Json<AcceptedEvent>), ApiError> {
    validate_event(&event)?;
    let receipt = state
        .store
        .accept(
            event,
            state.config.max_server_ttl,
            OffsetDateTime::now_utc(),
        )
        .await?;

    Ok((StatusCode::ACCEPTED, Json(receipt)))
}

async fn list_events(
    State(state): State<AppState>,
    Path(channel_id): Path<Uuid>,
    Query(cursor): Query<EventCursor>,
) -> Json<Vec<DeliveredEvent>> {
    Json(
        state
            .store
            .list_active(
                channel_id,
                cursor.after_sequence.unwrap_or_default(),
                OffsetDateTime::now_utc(),
            )
            .await,
    )
}

fn validate_event(event: &SubmitEvent) -> Result<(), ApiError> {
    if event.sender_counter == 0 {
        return Err(ApiError::unprocessable(
            "invalid_sender_counter",
            "sender counter must be positive",
        ));
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct EventCursor {
    after_sequence: Option<u64>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl ApiError {
    fn unprocessable(code: &'static str, message: &'static str) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code,
            message,
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::EventIdConflict(_) => Self {
                status: StatusCode::CONFLICT,
                code: "event_id_conflict",
                message: "event ID was already used for a different payload",
            },
            StoreError::SenderCounterReplay { .. } => Self {
                status: StatusCode::CONFLICT,
                code: "sender_counter_replay",
                message: "sender counter must advance monotonically",
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                code: self.code,
                message: self.message,
            }),
        )
            .into_response()
    }
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use http::Request;
    use serde_json::json;
    use singularis_mls::{EventContent, EventReferences, MlsClient, TextContent, VaultMlsClient};
    use singularis_protocol::{OpaqueMlsMessage, ServerTtl};
    use singularis_vault::Vault;
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn test_router() -> Router {
        router(
            AppConfig::default(),
            Arc::new(InMemoryEventStore::default()),
        )
    }

    fn event_body(event_id: Uuid, channel_id: Uuid, ttl_seconds: u64) -> String {
        json!({
            "event_id": event_id,
            "community_id": Uuid::new_v4(),
            "channel_id": channel_id,
            "sender_device_id": Uuid::new_v4(),
            "sender_counter": 1,
            "mls_message": OpaqueMlsMessage::from_bytes(b"opaque-test-mls-message").unwrap(),
            "ttl_seconds": ttl_seconds
        })
        .to_string()
    }

    #[tokio::test]
    async fn healthcheck_is_available() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn json_contract_rejects_ttl_over_seven_days() {
        let body = event_body(Uuid::new_v4(), Uuid::new_v4(), 604_801);
        let response = test_router()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/events")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn accepted_event_can_be_listed_by_channel() {
        let app = test_router();
        let channel_id = Uuid::new_v4();
        let submit_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/events")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(event_body(Uuid::new_v4(), channel_id, 300)))
                    .unwrap(),
            )
            .await
            .unwrap();

        let list_response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/channels/{channel_id}/events"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(submit_response.status(), StatusCode::ACCEPTED);
        assert_eq!(list_response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn opaque_mls_event_round_trips_between_two_clients() {
        let community_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let mut alice = MlsClient::new(Uuid::new_v4()).unwrap();
        let mut bob = MlsClient::new(Uuid::new_v4()).unwrap();
        let bob_key_package = bob.publish_key_package().unwrap();
        alice.create_channel(community_id, channel_id).unwrap();
        let addition = alice.add_member(channel_id, &bob_key_package).unwrap();
        bob.join_channel(community_id, channel_id, &addition.welcome)
            .unwrap();

        let canary = "HTTP-RELAY-MUST-NOT-SEE-THIS";
        let encrypted = alice
            .encrypt_text(
                channel_id,
                1_765_000_000_000,
                EventReferences::default(),
                canary.to_owned(),
            )
            .unwrap();
        let submit = encrypted
            .to_submit_event(singularis_protocol::ServerTtl::MIN)
            .unwrap();
        let submit_body = serde_json::to_vec(&submit).unwrap();
        assert!(
            !submit_body
                .windows(canary.len())
                .any(|window| window == canary.as_bytes())
        );

        let app = test_router();
        let submit_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/events")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(submit_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(submit_response.status(), StatusCode::ACCEPTED);

        let list_response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/channels/{channel_id}/events"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list_response.status(), StatusCode::OK);
        let delivered_body = to_bytes(list_response.into_body(), MAX_EVENT_BODY_BYTES)
            .await
            .unwrap();
        assert!(
            !delivered_body
                .windows(canary.len())
                .any(|window| window == canary.as_bytes())
        );
        let delivered: Vec<DeliveredEvent> = serde_json::from_slice(&delivered_body).unwrap();

        assert_eq!(delivered.len(), 1);
        assert_eq!(
            bob.decrypt_delivered_event(&delivered[0]).unwrap().content,
            EventContent::Text(TextContent {
                body: canary.to_owned()
            })
        );
    }

    #[tokio::test]
    async fn queued_mls_event_survives_restart_and_relay_retry() {
        let temporary_directory = tempdir().unwrap();
        let vault_root = temporary_directory.path().join("alice-vault");
        let mut vault = Vault::new(&vault_root);
        vault.initialize("correct horse battery staple").unwrap();
        let community_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let alice_device_id = Uuid::new_v4();
        let mut alice = VaultMlsClient::create(&vault, alice_device_id).unwrap();
        let mut bob = MlsClient::new(Uuid::new_v4()).unwrap();
        let bob_key_package = bob.publish_key_package().unwrap();
        alice.create_channel(community_id, channel_id).unwrap();
        let addition = alice.add_member(channel_id, &bob_key_package).unwrap();
        bob.join_channel(community_id, channel_id, &addition.welcome)
            .unwrap();

        let canary = "OUTBOX-CRASH-CANARY-STAYS-IN-MLS";
        let queued = alice
            .queue_text(
                channel_id,
                1_786_300_000_000,
                EventReferences::default(),
                canary.to_owned(),
                ServerTtl::MIN,
            )
            .unwrap();
        let queued_body = serde_json::to_vec(&queued).unwrap();
        assert!(
            !queued_body
                .windows(canary.len())
                .any(|part| part == canary.as_bytes())
        );
        drop(alice);
        vault.lock().unwrap();
        drop(vault);

        let mut reopened_vault = Vault::new(&vault_root);
        reopened_vault
            .unlock("correct horse battery staple")
            .unwrap();
        let alice = VaultMlsClient::load(&reopened_vault, alice_device_id)
            .unwrap()
            .unwrap();
        let pending = alice.pending_submissions().unwrap();
        assert_eq!(pending, vec![queued.clone()]);

        let app = test_router();
        let mut receipts = Vec::new();
        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/v1/events")
                        .header(CONTENT_TYPE, "application/json")
                        .body(Body::from(serde_json::to_vec(&pending[0]).unwrap()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::ACCEPTED);
            receipts.push(
                serde_json::from_slice::<AcceptedEvent>(
                    &to_bytes(response.into_body(), MAX_EVENT_BODY_BYTES)
                        .await
                        .unwrap(),
                )
                .unwrap(),
            );
        }
        assert_eq!(receipts[0], receipts[1]);

        let list_response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/channels/{channel_id}/events"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let delivered: Vec<DeliveredEvent> = serde_json::from_slice(
            &to_bytes(list_response.into_body(), MAX_EVENT_BODY_BYTES)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(
            bob.decrypt_delivered_event(&delivered[0]).unwrap().content,
            EventContent::Text(TextContent {
                body: canary.to_owned()
            })
        );

        assert!(alice.acknowledge_submission(receipts[0].event_id).unwrap());
        assert!(alice.pending_submissions().unwrap().is_empty());
        drop(alice);
        reopened_vault.lock().unwrap();
        drop(reopened_vault);

        let mut final_vault = Vault::new(&vault_root);
        final_vault.unlock("correct horse battery staple").unwrap();
        let alice = VaultMlsClient::load(&final_vault, alice_device_id)
            .unwrap()
            .unwrap();
        assert!(alice.pending_submissions().unwrap().is_empty());
    }
}
