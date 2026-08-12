use std::{env, net::SocketAddr};

use anyhow::{Context, Result, bail};
use singularis_protocol::ServerTtl;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub bind_addr: SocketAddr,
    pub max_server_ttl: ServerTtl,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let bind_addr = env::var("SINGULARIS_BIND_ADDR")
            .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned())
            .parse::<SocketAddr>()
            .context("SINGULARIS_BIND_ADDR must be a socket address")?;

        if !bind_addr.ip().is_loopback() {
            bail!(
                "the prototype server has no device authentication and only permits loopback binding"
            );
        }

        let max_server_ttl = match env::var("SINGULARIS_MAX_TTL_SECONDS") {
            Ok(value) => {
                let seconds = value
                    .parse::<u64>()
                    .context("SINGULARIS_MAX_TTL_SECONDS must be an integer")?;
                ServerTtl::try_from(seconds).context("invalid SINGULARIS_MAX_TTL_SECONDS")?
            }
            Err(env::VarError::NotPresent) => ServerTtl::MAX,
            Err(error) => return Err(error).context("could not read SINGULARIS_MAX_TTL_SECONDS"),
        };

        Ok(Self {
            bind_addr,
            max_server_ttl,
        })
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            bind_addr: DEFAULT_BIND_ADDR
                .parse()
                .expect("the built-in bind address must be valid"),
            max_server_ttl: ServerTtl::MAX,
        }
    }
}
