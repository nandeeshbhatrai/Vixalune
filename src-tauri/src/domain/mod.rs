//! Pure domain layer: types, validation and state machines. No I/O, no Tauri.

pub mod activity;
pub mod container;
pub mod environment;
pub mod errors;
pub mod failure;
pub mod ids;
pub mod install_state;
pub mod jobs;
pub mod lab;
pub mod learning;
pub mod manifest;
pub mod paths;
pub mod platform;
pub mod process;
pub mod runs;
pub mod scope;
pub mod settings;
pub mod storage;

pub use errors::{AppError, AppResult};
pub use ids::{LabId, OperationId, ProcessId, RunId, ToolId};



