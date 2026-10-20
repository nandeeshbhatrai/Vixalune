//! Diagnostic logging (developer-facing). Distinct from the user-facing activity log, which is
//! stored in SQLite and redacted. Never log command-line parameter values or secrets here.

use tracing_subscriber::EnvFilter;

pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,vixalune_lib=debug"));
    // `try_init` so repeated calls (tests) don't panic.
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_target(true).try_init();
    install_panic_hook();
}

/// Panic messages are logged through the same redaction as provider errors (no keys, no local paths in the message text) and
/// no report is written to disk or sent anywhere.
fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
            let at = info.location().map(|l| format!("{}:{}", l.file().rsplit(['\\', '/']).next().unwrap_or("?"), l.line())).unwrap_or_default();
            tracing::error!("internal error at {at}: {}", crate::domain::ai::sanitize_message(&msg, None));
        }));
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn panic_text_is_redacted_the_same_way_as_provider_errors() {
        let m = crate::domain::ai::sanitize_message("called unwrap on Err(\"bad key sk-abcdefghijklmnopqrstuvwx at C:\\Users\\me\\secret\\file.txt\")", None);
        assert!(!m.contains("sk-abcdef") && !m.contains("Users"), "{m}");
    }
}
