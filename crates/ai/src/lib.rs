//! AI assistance: talking to model providers and building what is sent to them.
//!
//! Requests are made from here, in the application's core, so API keys never
//! reach the interface. What a request contains is assembled in [`context`]
//! and [`prompt`]: the database's structure and the user's SQL, and never row
//! data unless a later feature adds it behind an explicit setting.

pub mod context;
pub mod prompt;
mod providers;
mod settings;
mod sse;

pub use providers::{
    ChatMessage, ChatRequest, Completion, ModelInfo, ProviderConfig, ProviderKind, Role,
    StopReason, StreamEvent, Usage, http_client, list_models, stream_chat,
};
pub use settings::{AiSettings, AiStore, ProviderEntry};
pub use tokio_util::sync::CancellationToken;

use dbclient_core::Error;

pub(crate) fn ai_error(reason: &str, message: impl Into<String>) -> Error {
    Error::Ai {
        reason: reason.to_owned(),
        message: message.into(),
    }
}
