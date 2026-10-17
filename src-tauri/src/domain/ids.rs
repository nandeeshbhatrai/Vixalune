//! Validated identifier newtypes. Identifiers reach file paths, database keys and process
//! arguments, so they are restricted to a conservative charset at construction time.

use super::errors::AppError;
use serde::{Deserialize, Serialize};
use std::fmt;

fn valid_slug(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

macro_rules! slug_id {
    ($name:ident, $label:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, AppError> {
                let value = value.into();
                if valid_slug(&value) {
                    Ok(Self(value))
                } else {
                    Err(AppError::Validation(format!(
                        "{} '{}' must be 1-63 chars of a-z, 0-9 or '-', starting with a letter or digit",
                        $label, value
                    )))
                }
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = AppError;
            fn try_from(v: String) -> Result<Self, AppError> {
                Self::new(v)
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> String {
                v.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

slug_id!(ToolId, "tool id");
slug_id!(LabId, "lab id");

/// Opaque identifiers for runtime operations (installs, runs, processes).
macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub uuid::Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(uuid::Uuid::new_v4())
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

uuid_id!(OperationId);
uuid_id!(RunId);
uuid_id!(ProcessId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_slug() {
        assert!(ToolId::new("nmap").is_ok());
        assert!(ToolId::new("burp-suite-2").is_ok());
    }

    #[test]
    fn rejects_dangerous_or_malformed_slugs() {
        for bad in ["", "-x", "A", "a b", "a/b", "..", "a;b", "a\\b", &"a".repeat(64)] {
            assert!(ToolId::new(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn deserialization_validates() {
        assert!(serde_json::from_str::<ToolId>("\"../etc\"").is_err());
        assert!(serde_json::from_str::<ToolId>("\"nmap\"").is_ok());
    }
}
