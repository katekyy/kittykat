use std::str::FromStr;

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

    pub(super) fn is_anonymous(&self) -> bool {
        matches!(self, Self::Anonymous(_))
    }
}

impl FromStr for Token {
    type Err = core::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::session(s))
    }
}
