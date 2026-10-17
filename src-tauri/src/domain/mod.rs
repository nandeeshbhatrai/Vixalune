//! Pure domain layer: types, validation and state machines. No I/O, no Tauri.

pub mod activity;
pub mod environment;
pub mod errors;
pub mod ids;
pub mod install_state;
pub mod lab;
pub mod manifest;
pub mod process;

pub use errors::{AppError, AppResult};
pub use ids::{LabId, OperationId, ProcessId, RunId, ToolId};
