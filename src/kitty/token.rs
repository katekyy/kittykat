use std::{convert::Infallible, str::FromStr};

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
    type Err = Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::session(s))
    }
}
