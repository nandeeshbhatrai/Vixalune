//! Concrete implementations of the ports in `crate::ports`.

pub mod activity_sqlite;
pub mod ai;
pub mod host_facts;
pub mod container;
pub mod container_runner;
pub mod elevation;
pub mod executor;
pub mod launch_router;
pub mod learning;
pub mod link_opener;
pub mod locator;
pub mod platform_detector;
pub mod native_runner;
pub mod practice_lab;
pub mod run_support;
pub mod settings_store;
pub mod registry;
pub mod resolver;
pub mod result_saver;
pub mod setup;
pub mod sqlite_store;
pub mod system_net;
#[cfg(test)]
pub mod testing;
pub mod unimplemented;
pub mod winget;
pub mod wsl;





