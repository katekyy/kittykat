use std::str::FromStr;

use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Token {
    Anonymous(Uuid),
    Session(String),
}

impl Token {
    pub(super) fn session<S>(s: S) -> Self
    where
        S: ToString,
    {
        Self::Session(s.to_string())
    }

    pub(super) fn anonymous() -> Self {
        Self::Anonymous(Uuid::new_v4())
    }
}

impl FromStr for Token {
    type Err = core::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::session(s))
    }
}

impl Serialize for Token {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Anonymous(uuid) => serializer.serialize_str(&format!("{}", uuid)),
            Self::Session(s) => serializer.serialize_str(s),
        }
    }
}
