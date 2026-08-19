use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! name_type {
    ($ty:ident, $what:literal, $valid:path) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $ty(String);

        impl $ty {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $ty {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                if $valid(&value) {
                    Ok(Self(value))
                } else {
                    Err(format!("invalid {}: {value:?}", $what))
                }
            }
        }

        impl std::str::FromStr for $ty {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::try_from(value.to_owned())
            }
        }

        impl From<$ty> for String {
            fn from(value: $ty) -> Self {
                value.0
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

name_type!(RoleName, "role name", is_name);
name_type!(AgentName, "agent name", is_name);
name_type!(WorkspaceName, "workspace name", is_name);
name_type!(EnvKey, "env key", is_env_key);
name_type!(Domain, "domain", is_domain);
name_type!(ImageRef, "image reference", is_image);
name_type!(Digest, "digest", is_digest);

impl Domain {
    pub fn covers(&self, host: &str) -> bool {
        match self.0.strip_prefix("*.") {
            Some(suffix) => host
                .strip_suffix(suffix)
                .is_some_and(|rest| rest.is_empty() || rest.ends_with('.')),
            None => self.0 == host,
        }
    }

    pub fn wildcard_suffix(&self) -> Option<&str> {
        self.0.strip_prefix("*.")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SecretRef {
    store: String,
    name: String,
}

impl SecretRef {
    pub fn store(&self) -> &str {
        &self.store
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl TryFrom<String> for SecretRef {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let err = || format!("invalid secret ref (expected reef://<store>/<name>): {value:?}");
        let rest = value.strip_prefix("reef://").ok_or_else(err)?;
        let (store, name) = rest.split_once('/').ok_or_else(err)?;
        if is_name(store) && is_name(name) {
            Ok(Self {
                store: store.to_owned(),
                name: name.to_owned(),
            })
        } else {
            Err(err())
        }
    }
}

impl From<SecretRef> for String {
    fn from(value: SecretRef) -> Self {
        value.to_string()
    }
}

impl std::str::FromStr for SecretRef {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "reef://{}/{}", self.store, self.name)
    }
}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && !s.ends_with('-')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_env_key(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn is_domain(s: &str) -> bool {
    let host = s.strip_prefix("*.").unwrap_or(s);
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
}

fn is_image(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 400
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._:/@+-".contains(c))
}

fn is_digest(s: &str) -> bool {
    s.len() == 64
        && s.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_reject_bad_shapes() {
        assert!(AgentName::try_from("reviewer-1".to_owned()).is_ok());
        for bad in ["", "-x", "x-", "X", "a b", "a_b", &"a".repeat(41)] {
            assert!(AgentName::try_from(bad.to_owned()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn env_keys() {
        assert!(EnvKey::try_from("ANTHROPIC_API_KEY".to_owned()).is_ok());
        assert!(EnvKey::try_from("lower".to_owned()).is_err());
        assert!(EnvKey::try_from("1X".to_owned()).is_err());
    }

    #[test]
    fn domains_and_wildcards() {
        let exact = Domain::try_from("api.anthropic.com".to_owned()).unwrap();
        assert!(exact.covers("api.anthropic.com"));
        assert!(!exact.covers("anthropic.com"));

        let wild = Domain::try_from("*.github.com".to_owned()).unwrap();
        assert!(wild.covers("raw.github.com"));
        assert!(wild.covers("a.b.github.com"));
        assert!(
            wild.covers("github.com"),
            "wildcards include the apex, matching enforcement"
        );
        assert!(!wild.covers("evilgithub.com"));

        assert!(Domain::try_from("bad..dot".to_owned()).is_err());
        assert!(Domain::try_from("-bad.com".to_owned()).is_err());
    }

    #[test]
    fn secret_refs() {
        let r = SecretRef::try_from("reef://platform/anthropic".to_owned()).unwrap();
        assert_eq!(r.store(), "platform");
        assert_eq!(r.name(), "anthropic");
        assert_eq!(r.to_string(), "reef://platform/anthropic");
        for bad in [
            "platform/anthropic",
            "reef://x",
            "reef://a/b/c",
            "reef://A/b",
        ] {
            assert!(SecretRef::try_from(bad.to_owned()).is_err(), "{bad:?}");
        }
    }
}
