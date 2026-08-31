use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! name_type {
    ($ty:ident, $what:literal, $valid:path) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
name_type!(VolumeName, "volume name", is_name);
name_type!(PortName, "port name", is_name);
name_type!(EnvKey, "env key", is_env_key);
name_type!(Domain, "domain", is_domain);
name_type!(Host, "host", is_host);
name_type!(ImageRef, "image reference", is_image);
name_type!(Digest, "digest", is_digest);
name_type!(GuestPath, "guest path", is_guest_path);

impl GuestPath {
    pub fn under(&self, dir: &Self) -> bool {
        self.0
            .strip_prefix(&dir.0)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }
}

impl Domain {
    pub fn is_any(&self) -> bool {
        self.0 == "*"
    }

    pub fn covers(&self, host: &str) -> bool {
        if self.is_any() {
            return true;
        }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        && !s.starts_with("REEF_")
        && s.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn is_domain(s: &str) -> bool {
    if s == "*" {
        return true;
    }
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

fn is_host(s: &str) -> bool {
    !s.contains('*') && is_domain(s)
}

fn is_image(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 400
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._:/@+-".contains(c))
}

fn is_guest_path(s: &str) -> bool {
    s.starts_with('/')
        && s.len() <= 255
        && !s.contains(['\\', '\0'])
        && s.split('/')
            .skip(1)
            .all(|part| !part.is_empty() && part != "." && part != "..")
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
        assert!("reviewer-1".parse::<AgentName>().is_ok());
        for bad in ["", "-x", "x-", "X", "a b", "a_b", &"a".repeat(41)] {
            assert!(bad.parse::<AgentName>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn env_keys() {
        assert!("ANTHROPIC_API_KEY".parse::<EnvKey>().is_ok());
        assert!("lower".parse::<EnvKey>().is_err());
        assert!("REEF_PORT_UI".parse::<EnvKey>().is_err());
        assert!("1X".parse::<EnvKey>().is_err());
    }

    #[test]
    fn domains_and_wildcards() {
        let exact: Domain = "api.anthropic.com".parse().unwrap();
        assert!(exact.covers("api.anthropic.com"));
        assert!(!exact.covers("anthropic.com"));

        let wild: Domain = "*.github.com".parse().unwrap();
        assert!(wild.covers("raw.github.com"));
        assert!(wild.covers("a.b.github.com"));
        assert!(
            wild.covers("github.com"),
            "wildcards include the apex, matching enforcement"
        );
        assert!(!wild.covers("evilgithub.com"));

        assert!("bad..dot".parse::<Domain>().is_err());
        assert!("-bad.com".parse::<Domain>().is_err());
    }

    #[test]
    fn hosts_reject_wildcards() {
        assert!("api.anthropic.com".parse::<Host>().is_ok());
        assert!("*.anthropic.com".parse::<Host>().is_err());
        assert!("bad..dot".parse::<Host>().is_err());
    }

    #[test]
    fn guest_paths_are_absolute_and_normalized() {
        assert!("/etc/agent/config.json".parse::<GuestPath>().is_ok());
        for bad in [
            "", "/", "etc/x", "/a/", "/a//b", "/a/../b", "/a/./b", "/a\\b", "/a\0b",
        ] {
            assert!(bad.parse::<GuestPath>().is_err(), "{bad:?}");
        }

        let path = |text: &str| text.parse::<GuestPath>().unwrap();
        let home = path("/root");
        assert!(path("/root").under(&home));
        assert!(path("/root/.config/app").under(&home));
        assert!(!path("/rootfs/x").under(&home));
        assert!(!path("/etc/x").under(&home));
    }

    #[test]
    fn secret_refs() {
        let r: SecretRef = "reef://platform/anthropic".parse().unwrap();
        assert_eq!(r.store(), "platform");
        assert_eq!(r.name(), "anthropic");
        assert_eq!(r.to_string(), "reef://platform/anthropic");
        for bad in [
            "platform/anthropic",
            "reef://x",
            "reef://a/b/c",
            "reef://A/b",
        ] {
            assert!(bad.parse::<SecretRef>().is_err(), "{bad:?}");
        }
    }
}
