//! Concrete implementations of the ports in `crate::ports`.

pub mod activity_sqlite;
pub mod env_detector;
pub mod executor;
pub mod link_opener;
pub mod locator;
pub mod registry;
pub mod resolver;
pub mod sqlite_store;
#[cfg(test)]
pub mod testing;
pub mod unimplemented;
pub mod winget;
