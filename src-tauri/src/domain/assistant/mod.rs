//! Domain rules for the tool-aware assistant: untrusted-text handling, package questions, vulnerability evidence, the typed
//! action set and the validation of what a model says. Pure: no I/O.

pub mod actions;
pub mod model;
pub mod packages;
pub mod untrusted;
pub mod vuln;
