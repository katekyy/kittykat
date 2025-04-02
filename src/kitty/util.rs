use bytes::Buf;
use http::Request;
use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use hyper::body::Incoming;

pub fn full<B: Buf + Send + Sync + 'static, T: Into<B>>(chunk: T) -> BoxBody<B, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

pub fn empty<B: Buf + 'static>() -> BoxBody<B, hyper::Error> {
    Empty::<B>::new().map_err(|never| match never {}).boxed()
}

pub fn extract_token(req: &Request<Incoming>) -> Option<String> {
    // The authorization scheme (Basic, Bearer, etc.) doesn't really stop us from using the entire string here.
    // ~ some silly snep
    let token = req
        .headers()
        .get("proxy-authorization")
        .and_then(|v| v.to_str().map(|s| String::from(s)).ok());
    dbg!(&token, req);
    token
}
