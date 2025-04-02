use std::{collections::HashMap, sync::Arc};

use futures::FutureExt;
use http::{Response, StatusCode};
use tracing::{error, trace};
use url::Url;

use super::{special::Hook, util, KittyKat, Token};

pub(super) const BUILTIN_HOOKS: [Hook; 1] = [BUILTIN_API_HOOK];

pub(super) const BUILTIN_API_HOOK: Hook = Hook {
    name: "api",
    callback: |kitty_kat, req| {
        trace!("Accessing kittykat api with req: {:?}", &req);

        async move {
            let uri = req.uri();
            let url = Url::parse(&uri.to_string()).expect("failed to parse requested URI");

            let command = match url.path() {
                "/sessions" => {
                    let query = url.query_pairs().collect::<HashMap<_, _>>();

                    let pretty = match query
                        .get("pretty")
                        .map(|x| x.parse::<bool>().ok())
                        .unwrap_or(Some(false))
                    {
                        Some(pretty) => pretty,
                        None => {
                            return Ok(Response::builder()
                                .status(StatusCode::BAD_REQUEST)
                                .body(util::empty())
                                .unwrap())
                        }
                    };

                    Command::GetSessions { pretty }
                }

                "/self/destroy" => {
                    let token = if let Some(token) = util::extract_token(&req) {
                        token
                    } else {
                        return Ok(Response::builder()
                            .status(StatusCode::BAD_REQUEST)
                            .body(util::empty())
                            .unwrap());
                    };

                    Command::DestroySelf { token }
                }

                "/self/address/timeout" => {
                    let token = if let Some(token) = util::extract_token(&req) {
                        token
                    } else {
                        return Ok(Response::builder()
                            .status(StatusCode::BAD_REQUEST)
                            .body(util::empty())
                            .unwrap());
                    };

                    Command::AddressTimeout { token }
                }

                _ => {
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(util::empty())
                        .unwrap())
                }
            };

            match process_command(kitty_kat, command).await {
                Ok(Message::Sessions(s)) => Ok(Response::new(util::full(s))),

                Ok(Message::Unit) => Ok(Response::new(util::empty())),

                Err(Error::SerializeError) => Ok(Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(util::empty())
                    .unwrap()),

                Err(Error::SessionNotFound) => Ok(Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(util::full("Requested session token is not registered yet!"))
                    .unwrap()),
            }
        }
        .boxed()
    },
};

enum Command {
    GetSessions { pretty: bool },
    AddressTimeout { token: String },
    DestroySelf { token: String },
}

enum Message {
    Unit,
    Sessions(String),
}

enum Error {
    SerializeError,
    SessionNotFound,
}

async fn process_command(kitty_kat: KittyKat, command: Command) -> Result<Message, Error> {
    use Command::*;

    match command {
        DestroySelf { token } => {
            let mut sesssions = kitty_kat.sessions.write().await;

            let token = Token::session(token);

            sesssions.remove(&token);
            timeout_session_address(kitty_kat.clone(), &token).await?;

            Ok(Message::Unit)
        }

        AddressTimeout { token } => {
            let token = Token::session(token);
            timeout_session_address(kitty_kat, &token).await?;
            Ok(Message::Unit)
        }

        GetSessions { pretty } => {
            let sessions = kitty_kat.sessions.read().await;
            let mut buf = HashMap::with_capacity(sessions.capacity());

            for (t, s) in sessions.iter() {
                buf.insert(t, s.lock().await.clone());
            }

            let to_json_string = if pretty {
                serde_json::to_string_pretty
            } else {
                serde_json::to_string
            };

            match to_json_string(&buf) {
                Ok(res) => Ok(Message::Sessions(res)),
                Err(err) => {
                    error!("Caught an error while serializing sessions: {}", err);
                    Err(Error::SerializeError)
                }
            }
        }
    }
}

async fn timeout_session_address(kitty_kat: KittyKat, token: &Token) -> Result<(), Error> {
    let sessions = kitty_kat.sessions.read().await;

    let session = sessions.get(token).cloned().ok_or(Error::SessionNotFound)?;

    let mut session_lock = session.lock().await;

    if let Some(addr) = session_lock.addr.take() {
        let mut addrs = kitty_kat.addrs.write().await;
        addrs.insert(addr, Arc::clone(&session));
    }

    Ok(())
}
