//! Concrete implementations of the ports in `crate::ports`.

pub mod activity_sqlite;
pub mod host_facts;
pub mod container;
pub mod container_runner;
pub mod elevation;
pub mod executor;
pub mod link_opener;
pub mod locator;
pub mod platform_detector;
pub mod registry;
pub mod resolver;
pub mod setup;
pub mod sqlite_store;
#[cfg(test)]
pub mod testing;
pub mod unimplemented;
pub mod winget;
pub mod wsl;




