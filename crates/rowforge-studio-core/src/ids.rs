//! Strong newtypes for execution and attempt identifiers.
//!
//! Studio uses these to prevent crossed args at call sites
//! (`StudioCore::attempt(exec, attempt)` is hard to swap). CLI continues
//! to use bare `String` IDs; conversion happens at the Tauri command
//! boundary.
//!
//! Spec: `docs/spec/studio/part-2-model.md` §2.1 entity inventory
//! (`ExecutionId`, `AttemptId` are conceptual types in the spec; this
//! module gives them concrete Rust shape).

use serde::{Deserialize, Serialize};

macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_newtype!(ExecutionId);
id_newtype!(AttemptId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_serialize_transparently_as_string() {
        let e = ExecutionId::new("e1");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v, serde_json::Value::String("e1".into()));
    }

    #[test]
    fn ids_deserialize_from_string() {
        let e: ExecutionId = serde_json::from_str(r#""e1""#).unwrap();
        assert_eq!(e.as_str(), "e1");
    }

    #[test]
    fn execution_and_attempt_are_distinct_types() {
        let e = ExecutionId::new("e1");
        let a = AttemptId::new("a1");
        assert_ne!(e.as_str(), a.as_str());
    }
}
