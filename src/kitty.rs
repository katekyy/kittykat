use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use arti_client::{DataStream, IsolationToken, StreamPrefs, TorClient};
use bytes::Bytes;
use http::{HeaderValue, Method, Request, Response, StatusCode};
use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use hyper::{
    body::Incoming,
    rt::{Read, Write},
    server::conn::http1,
    service::service_fn,
    upgrade::Upgraded,
};
use hyper_util::rt::TokioIo;
use token::Token;
use tokio::{io, sync::Mutex};
use tokio_task_pool::Task;
use tor_rtcompat::tokio::PreferredRuntime;
use tracing::{debug, error, info, trace, warn};

mod token;

type IsolationPool = HashMap<Token, Isolation>;

#[derive(Clone, Copy)]
struct Isolation {
    isolation: IsolationToken,
    last_used: Instant,
}

#[derive(Clone, Debug, Default)]
pub struct Preferences {
    pub client_lifetime: Duration,
    pub pool_bound: Option<usize>,
    pub stream_prefs: StreamPrefs,
}

#[derive(Clone)]
pub struct KittyKat {
    isolation_pool: Arc<Mutex<IsolationPool>>,
    client: Arc<TorClient<PreferredRuntime>>,
    task_pool: Arc<tokio_task_pool::Pool>,
    preferences: Preferences,
}

impl KittyKat {
    pub async fn new(client: TorClient<PreferredRuntime>, prefs: Preferences) -> Self {
        info!("Creating a KittyKat instance with preferences: {prefs:?}");

        let task_pool = if let Some(bound) = prefs.pool_bound {
            tokio_task_pool::Pool::bounded(bound + 1)
        } else {
            tokio_task_pool::Pool::unbounded()
        };

        let task_pool = Arc::new(task_pool);

        let s = Self {
            isolation_pool: Arc::new(Mutex::new(HashMap::new())),
            client: Arc::new(client),
            task_pool: Arc::clone(&task_pool),
            preferences: prefs,
        };

        let s_clone = s.clone();
        task_pool
            .spawn_task(
                Task::new(async move {
                    let s = s_clone.clone();
                    s.cleanup_stale_isolations().await;
                })
                .with_id("cleanup"),
            )
            .await
            .unwrap();

        s
    }

    pub async fn serve_connection<I>(&self, io: I)
    where
        I: Read + Write + Unpin + Send + 'static,
    {
        let self_clone = self.clone();
        let service = service_fn(move |req| Self::proxy(self_clone.clone(), req));
        self.task_pool
            .spawn(async move {
                match http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades()
                    .await
                {
                    Ok(()) => {}
                    Err(err) => eprintln!("Got an error while serving a connection: {}", err),
                }
            })
            .await
            .unwrap();
    }

    async fn proxy(
        self,
        req: Request<Incoming>,
    ) -> Result<Response<BoxBody<hyper::body::Bytes, hyper::Error>>, hyper::Error> {
        trace!("Handling a new request: {:#?}", &req);

        if req.method() == Method::CONNECT {
            self.handle_tunnel(req).await
        } else {
            self.handle_unsafe_request(req).await
        }
    }

    async fn cleanup_stale_isolations(&self) {
        let mut interval = tokio::time::interval(self.preferences.client_lifetime);
        loop {
            interval.tick().await;
            let mut pool = self.isolation_pool.lock().await;
            pool.retain(|t, circuit| {
                let retain = circuit.last_used.elapsed() < self.preferences.client_lifetime;
                if !retain {
                    debug!("Purging isolation {:?}", t);
                }
                retain
            });
        }
    }

    async fn get_or_isolate(&self, maybe_token: Option<String>) -> Option<IsolationToken> {
        let mut pool = self.isolation_pool.lock().await;

        let token = match maybe_token {
            Some(token) => {
                let token = Token::session(token);

                // Create a new circuit if the last one got cleaned up or it never existed in the first place.
                if let Some(circuit) = pool.get_mut(&token) {
                    circuit.last_used = Instant::now();
                    return Some(circuit.isolation);
                }

                token
            }
            None => Token::anonymous(),
        };

        let isolation = IsolationToken::new();
        pool.insert(
            token,
            Isolation {
                isolation,
                last_used: Instant::now(),
            },
        );
        Some(isolation)
    }

    fn extract_token(req: &Request<Incoming>) -> Option<String> {
        // The authorization scheme (Basic, Bearer, etc.) doesn't really stop us from using the entire string here.
        // ~ some silly snep
        let token = req
            .headers()
            .get("proxy-authorization")
            .and_then(|v| v.to_str().map(|s| String::from(s)).ok());
        token
    }

    async fn handle_tunnel(
        &self,
        req: Request<Incoming>,
    ) -> Result<Response<BoxBody<hyper::body::Bytes, hyper::Error>>, hyper::Error> {
        let uri = req.uri().clone();
        let (host, port) = (
            uri.host().expect("absolute URI").to_string(),
            uri.port_u16().unwrap_or(443),
        );

        let isolation = match self.get_or_isolate(Self::extract_token(&req)).await {
            Some(isolation) => isolation,
            None => {
                return Ok(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body(full("Invalid token"))
                    .unwrap());
            }
        };

        let client = Arc::clone(&self.client);
        let mut stream_prefs = self.preferences.stream_prefs.clone();
        tokio::spawn(async move {
            stream_prefs.set_isolation(isolation);

            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    match client.connect_with_prefs((host, port), &stream_prefs).await {
                        Ok(tor_stream) => {
                            if let Err(err) = Self::tunnel(upgraded, tor_stream).await {
                                error!("Tunnel failed: {}", err);
                            }
                        }
                        Err(err) => error!("Tor connection failed: {}", err),
                    }
                }
                Err(err) => error!("Upgrade failed: {}", err),
            }
        });

        Ok(Response::new(empty()))
    }

    async fn tunnel(upgraded: Upgraded, mut tor_stream: DataStream) -> io::Result<()> {
        let mut client_stream = TokioIo::new(upgraded);

        let (_client_count, _target_count) =
            tokio::io::copy_bidirectional(&mut client_stream, &mut tor_stream).await?;

        // let (mut client_read, mut client_write) = tokio::io::split(&mut client_stream);
        // let (mut target_read, mut target_write) = tokio::io::split(&mut tor_stream);

        // // Make each direction shutdown on it's own.
        // let client_to_tor = async {
        //     let bytes = tokio::io::copy(&mut client_read, &mut target_write).await?;
        //     target_write.shutdown().await?;
        //     Ok::<u64, io::Error>(bytes)
        // };

        // let tor_to_client = async {
        //     let bytes = tokio::io::copy(&mut target_read, &mut client_write).await?;
        //     client_write.shutdown().await?;
        //     Ok::<u64, io::Error>(bytes)
        // };

        // let (client_res, tor_res) = tokio::join!(client_to_tor, tor_to_client);

        // client_res?;
        // tor_res?;

        Ok(())
    }

    async fn handle_unsafe_request(
        &self,
        req: Request<Incoming>,
    ) -> Result<Response<BoxBody<hyper::body::Bytes, hyper::Error>>, hyper::Error> {
        let uri = req.uri().clone();
        let (host, port) = (
            uri.host().expect("absolute URI"),
            uri.port_u16().unwrap_or(80),
        );

        let isolation = match self.get_or_isolate(Self::extract_token(&req)).await {
            Some(isolation) => isolation,
            None => {
                return Ok(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body(full("Invalid token"))
                    .unwrap());
            }
        };

        debug!("Attempting an unsafe connection to: {}", uri);

        // Connect through Tor owo.
        let mut stream_prefs = self.preferences.stream_prefs.clone();
        stream_prefs.set_isolation(isolation);

        let stream = match self
            .client
            .connect_with_prefs((host, port), &stream_prefs)
            .await
        {
            Ok(s) => TokioIo::new(s),
            Err(err) => {
                warn!("Tor connection failed: {}", err);
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(full(format!("Tor connection failed: {}", err)))
                    .unwrap());
            }
        };

        // Perform a HTTP handshake.
        let (mut sender, conn) = hyper::client::conn::http1::handshake(stream).await?;

        tokio::spawn(async move {
            if let Err(err) = conn.await {
                eprintln!("Connection error: {}", err);
            }
        });

        // Convert request for forwarding.
        let (parts, body) = req.into_parts();
        let path = parts
            .uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/");

        let mut outgoing_req = Request::builder()
            .method(parts.method)
            .uri(path)
            .version(parts.version);

        *outgoing_req.headers_mut().unwrap() = parts.headers.clone();
        outgoing_req
            .headers_mut()
            .unwrap()
            .insert("host", HeaderValue::from_str(host).unwrap());

        let outgoing_req = outgoing_req.body(body).unwrap();

        // Forward request and relay response.
        let mut response = sender.send_request(outgoing_req).await?;
        let status = response.status();
        let version = response.version();

        let headers = std::mem::take(response.headers_mut());

        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();

        let mut response = Response::builder()
            .status(status)
            .version(version)
            .body(full(body_bytes))
            .unwrap();

        response.headers_mut().extend(headers);

        Ok(response)
    }
}

fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

fn empty() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}
