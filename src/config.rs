use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    net::SocketAddrV4,
    str::FromStr,
    time::Duration,
};

use serde::{Deserialize, Serialize};

pub const CONFIG_FILE: &'static str = "kittykat.toml";

const LISTEN_ADDRESS: &'static str = "KITTYKAT_LISTEN_ADDRESS";
const SESSION_LIFETIME: &'static str = "KITTYKAT_SESSION_LIFETIME";
const ADDRESS_TIMEOUT: &'static str = "KITTYKAT_ADDRESS_TIMEOUT";
const OPTIMISTIC_STREAM: &'static str = "KITTYKAT_OPTIMISTIC";
const LOG_LEVEL: &'static str = "KITTYKAT_LOG";
const MAX_CIRCUIT_DIRTINESS: &'static str = "KITTYKAT_MAX_CIRC_DIRT";

// A cursed macro to make my life easier. Or harder?
macro_rules! get_var {
    ($vars:expr, $name:ident else $err:literal default $default:expr) => {
        $vars.get($name).map_or(Ok($default), |v| {
            FromStr::from_str(&v).map_err(|_| Error::ParseError(String::from($err)))
        })
    };
    ($vars:expr, $name:ident else #$err:literal default $default:expr) => {
        $vars.get($name).map_or(Ok($default), |v| {
            FromStr::from_str(&v).map_err(|_| Error::ParseError(format!($err, &v)))
        })
    };
    ($vars:expr, $name:ident else $err:literal default) => {
        $vars.get($name).map_or(Ok(Default::default()), |v| {
            FromStr::from_str(&v).map_err(|_| Error::ParseError(String::from($err)))
        })
    };
    ($vars:expr, $name:ident else #$err:literal default) => {
        $vars.get($name).map_or(Ok(Default::default()), |v| {
            FromStr::from_str(&v).map_err(|_| Error::ParseError(format!($err, &v)))
        })
    };
    ($vars:expr, $name:ident else $err:literal) => {
        $vars
            .get($name)
            .map_or(Err(Error::MissingEnvironmentVariable($name)), |v| {
                FromStr::from_str(&v).map_err(|_| Error::ParseError(String::from($err)))
            })
    };
    ($vars:expr, $name:ident else #$err:literal) => {
        $vars
            .get($name)
            .map_or(Err(Error::MissingEnvironmentVariable($name)), |v| {
                FromStr::from_str(&v).map_err(|_| Error::ParseError(format!($err, &v)))
            })
    };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub listen_address: SocketAddrV4,

    #[serde(default = "default_session_lifetime")]
    session_lifetime: u64,

    #[serde(default = "default_address_timeout")]
    address_timeout: u64,

    #[serde(default)]
    pub optimistic_stream: bool,

    #[serde(default)]
    log_level: Level,

    #[serde(default = "default_circuit_dirtiness")]
    max_circuit_dirtiness: u64,
}

impl Config {
    pub fn from_toml(s: String) -> Result<Self, toml::de::Error> {
        toml::de::from_str(&s)
    }

    pub fn from_env(vars: &HashMap<String, String>) -> Result<Self, Error> {
        let listen_address =
            get_var!(vars, LISTEN_ADDRESS else #r#"listen address "{}" could not be parsed"#)?;
        let session_lifetime = get_var!(vars, SESSION_LIFETIME else #r#"session lifetime "{}" could not be parsed as an integer"# default default_session_lifetime())?;
        let address_timeout = get_var!(vars, ADDRESS_TIMEOUT else #r#"address lifetime "{}" could not be parsed as an integer"# default default_address_timeout())?;
        let optimistic_stream = get_var!(vars, OPTIMISTIC_STREAM else #r#"value "{}" for optimistic stream could not be parsed as a boolean"# default)?;
        let log_level = get_var!(vars, LOG_LEVEL else #r#"expected the log level to be one of: [trace, debug, info, warn, error], but got "{}" instead"# default)?;
        let max_circuit_dirtiness = get_var!(vars, MAX_CIRCUIT_DIRTINESS else #r#"max circuit dirtiness "{}" could not be parsed as an integer"# default default_circuit_dirtiness())?;

        Ok(Self {
            listen_address,
            session_lifetime,
            address_timeout,
            optimistic_stream,
            log_level,
            max_circuit_dirtiness,
        })
    }

    pub fn session_lifetime(&self) -> Duration {
        Duration::from_millis(self.session_lifetime)
    }

    pub fn log_level(&self) -> tracing::Level {
        use tracing::Level as tl;
        use Level::*;

        match self.log_level {
            Trace => tl::TRACE,
            Debug => tl::DEBUG,
            Info => tl::INFO,
            Warn => tl::WARN,
            Error => tl::ERROR,
        }
    }

    pub fn max_circuit_dirtiness(&self) -> Duration {
        Duration::from_millis(self.max_circuit_dirtiness)
    }
}

fn default_session_lifetime() -> u64 {
    10_000
}

fn default_address_timeout() -> u64 {
    default_session_lifetime() * 2
}

fn default_circuit_dirtiness() -> u64 {
    15_000
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Error {
    MissingEnvironmentVariable(&'static str),
    ParseError(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingEnvironmentVariable(name) => {
                f.write_str("missing environment variable: ")?;
                f.write_str(name)?;
                Ok(())
            }
            Self::ParseError(msg) => f.write_str(&msg),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Level {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl FromStr for Level {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "trace" => Ok(Self::Trace),
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "warn" => Ok(Self::Warn),
            "error" => Ok(Self::Error),
            _ => Err(()),
        }
    }
}
