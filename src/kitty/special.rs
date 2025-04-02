use futures::future::BoxFuture;
use http::{Request, Response};
use http_body_util::combinators::BoxBody;
use hyper::body::Incoming;

use super::KittyKat;

type HookCallback = fn(
    KittyKat,
    Request<Incoming>,
) -> BoxFuture<
    'static,
    Result<Response<BoxBody<hyper::body::Bytes, hyper::Error>>, hyper::Error>,
>;

/// Hooks are accessed by sending a request to `"{name}.kittykat.hook"` and proxying the connection with `kittykat`.
#[derive(Debug)]
pub struct Hook {
    pub name: &'static str,
    pub callback: HookCallback,
}

impl Hook {
    pub(super) fn match_host(&self, hostname: &str) -> bool {
        let mut buf = String::from(self.name);
        buf.push_str(".kittykat.hook");
        hostname == buf
    }
}
