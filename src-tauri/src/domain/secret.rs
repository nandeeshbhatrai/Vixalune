//! A secret value (an API key) that cannot be printed, serialized or cloned by accident.
//!
//! The only way to read it is `expose`, which the code that builds an HTTP header calls. `Debug` shows a mask, there is no
//! `Display`, no `Serialize` and no `Clone`, and the memory is overwritten when the value is dropped.

use zeroize::Zeroize;

pub struct Secret(String);

impl Secret {
    /// A placeholder for requests that carry no credential at all (public APIs).
    pub fn none() -> Self {
        Self(String::new())
    }

    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The raw value. Use it only to build the request that needs it; never log, store or return it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// A safe hint for the UI: the first characters of a *recognised* prefix and the length, never anything secret-bearing.
    pub fn hint(&self) -> String {
        const PREFIXES: &[&str] = &["sk-ant-", "sk-or-v1-", "sk-proj-", "sk-", "AIza"];
        let prefix = PREFIXES.iter().find(|p| self.0.starts_with(**p)).copied().unwrap_or("");
        format!("{prefix}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022} ({} characters)", self.0.chars().count())
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_prints() {
        let s = Secret::new("sk-ant-api03-very-secret-value".into());
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert_eq!(format!("{:?}", Some(&s)), "Some(Secret(***))");
        assert!(!s.hint().contains("very-secret"));
        assert!(s.hint().starts_with("sk-ant-"));
        assert!(s.hint().contains("(30 characters)"));
        assert_eq!(s.expose(), "sk-ant-api03-very-secret-value");
    }

    #[test]
    fn the_hint_of_an_unrecognised_key_shows_no_characters_at_all() {
        let s = Secret::new("abcdef123456".into());
        assert!(!s.hint().contains("abc"));
    }
}
