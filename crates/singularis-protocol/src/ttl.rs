use std::{fmt, time::Duration};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

pub const MIN_SERVER_TTL_SECONDS: u32 = 5 * 60;
pub const MAX_SERVER_TTL_SECONDS: u32 = 7 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ServerTtl(u32);

impl ServerTtl {
    pub const MIN: Self = Self(MIN_SERVER_TTL_SECONDS);
    pub const MAX: Self = Self(MAX_SERVER_TTL_SECONDS);

    pub const fn as_seconds(self) -> u32 {
        self.0
    }

    pub const fn as_duration(self) -> Duration {
        Duration::from_secs(self.0 as u64)
    }
}

impl Default for ServerTtl {
    fn default() -> Self {
        Self::MAX
    }
}

impl TryFrom<u64> for ServerTtl {
    type Error = TtlError;

    fn try_from(seconds: u64) -> Result<Self, Self::Error> {
        let seconds = u32::try_from(seconds).map_err(|_| TtlError { seconds })?;

        if !(MIN_SERVER_TTL_SECONDS..=MAX_SERVER_TTL_SECONDS).contains(&seconds) {
            return Err(TtlError {
                seconds: u64::from(seconds),
            });
        }

        Ok(Self(seconds))
    }
}

impl From<ServerTtl> for u64 {
    fn from(ttl: ServerTtl) -> Self {
        u64::from(ttl.0)
    }
}

impl fmt::Display for ServerTtl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}s", self.0)
    }
}

impl Serialize for ServerTtl {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u32(self.0)
    }
}

impl<'de> Deserialize<'de> for ServerTtl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let seconds = u64::deserialize(deserializer)?;
        Self::try_from(seconds).map_err(de::Error::custom)
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
#[error(
    "server TTL must be between {MIN_SERVER_TTL_SECONDS} and {MAX_SERVER_TTL_SECONDS} seconds, got {seconds}"
)]
pub struct TtlError {
    pub seconds: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_boundary_values() {
        assert_eq!(
            ServerTtl::try_from(u64::from(MIN_SERVER_TTL_SECONDS)),
            Ok(ServerTtl::MIN)
        );
        assert_eq!(
            ServerTtl::try_from(u64::from(MAX_SERVER_TTL_SECONDS)),
            Ok(ServerTtl::MAX)
        );
    }

    #[test]
    fn rejects_values_outside_the_policy() {
        assert!(ServerTtl::try_from(u64::from(MIN_SERVER_TTL_SECONDS - 1)).is_err());
        assert!(ServerTtl::try_from(u64::from(MAX_SERVER_TTL_SECONDS) + 1).is_err());
        assert!(ServerTtl::try_from(u64::MAX).is_err());
    }

    #[test]
    fn serde_rejects_an_invalid_ttl() {
        let error = serde_json::from_str::<ServerTtl>("604801").unwrap_err();
        assert!(error.to_string().contains("server TTL must be between"));
    }
}
