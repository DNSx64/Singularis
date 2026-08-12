#![forbid(unsafe_code)]

mod event;
mod ttl;

pub use event::{
    AcceptedEvent, DeliveredEvent, EventId, HealthResponse, MAX_MLS_MESSAGE_BYTES,
    OpaqueMlsMessage, OpaqueMlsMessageError, SubmitEvent,
};
pub use ttl::{MAX_SERVER_TTL_SECONDS, MIN_SERVER_TTL_SECONDS, ServerTtl, TtlError};
