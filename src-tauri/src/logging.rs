//! Diagnostic logging (developer-facing). Distinct from the user-facing activity log, which is
//! stored in SQLite and redacted. Never log command-line parameter values or secrets here.

use tracing_subscriber::EnvFilter;

pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,vixalune_lib=debug"));
    // `try_init` so repeated calls (tests) don't panic.
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_target(true).try_init();
}
