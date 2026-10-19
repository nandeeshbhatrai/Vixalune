//! The tool-aware assistant: retrieval from the real tool registry, public vulnerability sources, strictly validated actions, and
//! orchestration over the AI gateway. See docs/ASSISTANT.md.

pub mod context;
pub mod executor;
pub mod facts;
pub mod osv;
pub mod service;
#[cfg(test)]
pub mod test_support;
#[cfg(test)]
mod assistant_tests;
