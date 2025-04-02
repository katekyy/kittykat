use std::{any::type_name, collections::HashMap, io, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::anyhow;
use arti_client::{
    config::HasChanMethod, DataStream, IntoTorAddr, IsolationToken, StreamPrefs, TorClient,
};
use builtin::BUILTIN_HOOKS;
use futures::lock::Mutex;
use http::{HeaderValue, Method, Request, Response, StatusCode};
use http_body_util::{combinators::BoxBody, BodyExt};
use hyper::{
    body::Incoming,
    rt::{Read, Write},
    server::conn::http1,
    service::service_fn,
    upgrade::Upgraded,
};
use hyper_util::rt::TokioIo;
use serde::{ser::SerializeStruct, Serialize};
use special::Hook;
use token::Token;
use tokio::{
    sync::RwLock,
    time::{interval, Instant},
};
use tokio_task_pool::Task;
use tor_proto::stream::ClientStreamCtrl;
use tor_rtcompat::tokio::TokioNativeTlsRuntime;
use tracing::{debug, error, info, trace, warn};

mod builtin;
pub mod special;
mod token;
mod util;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Ready,
    Running,
}

#[derive(Debug, Clone)]
pub struct Session {
    addr: Option<SocketAddr>,
    isolation_token: IsolationToken,
    last_used: Instant,
}

impl Serialize for Session {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut s = serializer.serialize_struct("Session", 3)?;
        s.serialize_field("addr", &self.addr)?;
        s.serialize_field("lifetime", &self.last_used.elapsed().as_millis())?;
        s.end()
    }
}

#[derive(Debug, Clone)]
pub struct Preferences {
    pub stream: StreamPrefs,
    pub session_lifetime: Duration,
    pub addr_dirtyness: Duration,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            stream: StreamPrefs::default(),
            session_lifetime: Duration::from_secs(10),
            addr_dirtyness: Duration::from_secs(20),
        }
    }
}

#[derive(Clone)]
pub struct KittyKat {
    task_pool: Arc<tokio_task_pool::Pool>,

    addrs: Arc<RwLock<HashMap<SocketAddr, Arc<Mutex<Session>>>>>,

    sessions: Arc<RwLock<HashMap<Token, Arc<Mutex<Session>>>>>,

    tor_client: Arc<TorClient<TokioNativeTlsRuntime>>,

    prefs: Arc<Preferences>,

    hooks: Arc<[Hook]>,

    status: Arc<RwLock<Status>>,
}

impl KittyKat {
    pub fn new(
        client: TorClient<TokioNativeTlsRuntime>,
        prefs: Preferences,
        hooks: Option<Vec<Hook>>,
    ) -> Self {
        let task_pool = tokio_task_pool::Pool::bounded(num_cpus::get() * 2 + 1);

        let mut hooks = hooks.unwrap_or_default();
        hooks.extend(BUILTIN_HOOKS);

        Self {
            task_pool: Arc::new(task_pool),
            addrs: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            tor_client: Arc::new(client),
            prefs: Arc::new(prefs),
            hooks: Arc::from(hooks),
            status: Arc::new(RwLock::new(Status::Ready)),
        }
    }

    pub async fn start(&mut self) {
        *self.status.write().await = Status::Running;
        self.task_pool
            .spawn_task(Task::new(Self::cleanup(self.clone())).with_id("cleanup"))
            .await
            .expect("failed to start the cleanup task");
    }

    async fn cleanup(self) {
        let mut interval = interval(Duration::from_secs(1));
        let session_lifetime = self.prefs.session_lifetime;
        let addr_lifetime = self.prefs.addr_dirtyness;

        loop {
            let tick = interval.tick().await;

            let mut sessions = self.sessions.write().await;

            for (token, session) in sessions
                .iter()
                .map(|(k, v)| (k.clone(), Arc::clone(&v)))
                .collect::<Vec<_>>()
            {
                let elapsed = tick.duration_since(session.lock().await.last_used);

                if elapsed >= session_lifetime {
                    debug!("Removing session {:?} after {:?}.", token, elapsed);
                    sessions.remove(&token);
                }
            }

            drop(sessions);

            let mut addrs = self.addrs.write().await;

            for (addr, session) in addrs
                .iter()
                .map(|(k, v)| (k.clone(), Arc::clone(&v)))
                .collect::<Vec<_>>()
            {
                let elapsed = tick.duration_since(session.lock().await.last_used);

                if elapsed >= addr_lifetime {
                    debug!("Removing address {:?}", &addr);
                    addrs.remove(&addr);
                }
            }
        }
    }

    pub async fn serve_connection<I>(&self, io: I)
    where
        I: Read + Write + Unpin + Send + 'static,
    {
        if *self.status.read().await == Status::Ready {
            panic!(
                "tried to serve connection with {} in a ready state",
                type_name::<Self>()
            )
        }

        let self_clone = self.clone();
        let service = service_fn(move |req| Self::proxy_connection(self_clone.clone(), req));
        self.task_pool
            .spawn(async move {
                match http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades() // MANDATORY to handle tunnels
                    .await
                {
                    Ok(()) => {}
                    Err(err) => eprintln!("Got an error while serving a connection: {}", err),
                }
            })
            .await
            .unwrap();
    }

    async fn commit_address(&self, addr: SocketAddr) -> bool {
        let addrs = self.addrs.read().await;
        if addrs.contains_key(&addr) {
            false
        } else {
            true
        }
    }

    async fn fetch_session(&self, maybe_token: Option<String>) -> (Token, Arc<Mutex<Session>>) {
        let mut sessions = self.sessions.write().await;

        let (t, s) = if let Some(s) = maybe_token {
            let token = Token::session(s);

            if let Some(session) = sessions.get(&token).cloned() {
                (token, session)
            } else {
                let session = Arc::new(Mutex::new(Session {
                    addr: None,
                    isolation_token: IsolationToken::new(),
                    last_used: Instant::now(),
                }));

                (token, session)
            }
        } else {
            (
                Token::anonymous(),
                Arc::new(Mutex::new(Session {
                    addr: None,
                    isolation_token: IsolationToken::new(),
                    last_used: Instant::now(),
                })),
            )
        };

        sessions.insert(t.clone(), Arc::clone(&s));

        (t, s)
    }

    async fn proxy_connection(
        self,
        req: Request<Incoming>,
    ) -> Result<Response<BoxBody<hyper::body::Bytes, hyper::Error>>, hyper::Error> {
        trace!("Handling a new request: {:#?}", &req);

        if let Some(host) = req.uri().host() {
            if let Some(hook) = self.hooks.iter().find(|hook| hook.match_host(host)) {
                return (hook.callback)(self, req).await;
            }
        }

        if req.method() == Method::CONNECT {
            self.handle_tunnel(req).await
        } else {
            self.handle_unsafe_request(req).await
        }
    }

    async fn try_connect_individual<A>(
        &self,
        addr: A,
        prefs: &mut StreamPrefs,
        session: Arc<Mutex<Session>>,
        mut max_retries: u32,
    ) -> Result<arti_client::DataStream, anyhow::Error>
    where
        A: IntoTorAddr + Clone + Send,
    {
        while max_retries > 0 {
            let mut locked_session = session.lock().await;
            prefs.set_isolation(locked_session.isolation_token);

            let stream = self
                .tor_client
                .connect_with_prefs(addr.clone(), prefs)
                .await?;
            if let Some(new_exit) = Self::get_stream_address(&stream) {
                if let Some(stored) = locked_session.addr {
                    if new_exit != stored {
                        locked_session.addr = Some(new_exit);
                    }
                    locked_session.last_used = Instant::now();
                    return Ok(stream);
                } else {
                    // No stored exit - try to commit to this one.
                    if self.commit_address(new_exit.clone()).await {
                        let mut addrs = self.addrs.write().await;
                        addrs.insert(new_exit.clone(), Arc::clone(&session));

                        locked_session.addr = Some(new_exit);
                        locked_session.last_used = Instant::now();
                        return Ok(stream);
                    }
                }
            }

            max_retries -= 1;
        }
        Err(anyhow!("max retries reached for individual connection"))
    }

    fn get_stream_address(stream: &DataStream) -> Option<SocketAddr> {
        let circ = stream
            .client_stream_ctrl()
            .expect("failed to get client stream ctrl?!")
            .circuit()
            .expect("failed to get client circuit?!");

        let path_ref = circ.path_ref();
        let last_hop = path_ref.iter().last();

        if let Some(last_hop) = last_hop {
            let chan_target = last_hop
                .as_chan_target()
                .expect("channel target is virtual??");
            let chan_method = chan_target.chan_method();
            let addr = chan_method.socket_addrs().and_then(|addrs| {
                addrs
                    .iter()
                    .find(|a| matches!(a, std::net::SocketAddr::V4(_)))
            });

            if let None = addr {
                error!("No IPv4 address found for exit node.")
            }

            addr.copied()
        } else {
            None
        }
    }

    async fn handle_tunnel(
        &self,
        req: Request<Incoming>,
    ) -> Result<Response<BoxBody<hyper::body::Bytes, hyper::Error>>, hyper::Error> {
        let uri = req.uri().clone();

        trace!("Attempting to create a tunnel to: {}", uri);

        let (host, port) = (
            uri.host().expect("absolute URI").to_string(),
            uri.port_u16().unwrap_or(443),
        );

        let (_token, session) = self.fetch_session(util::extract_token(&req)).await;

        let mut stream_prefs = self.prefs.stream.clone();

        let kittykat = self.clone();

        let main_task = async move {
            let maybe_stream = kittykat
                .try_connect_individual((host.clone(), port), &mut stream_prefs, session, 32)
                .await;

            match maybe_stream {
                Ok(stream) => match hyper::upgrade::on(req).await {
                    Ok(upgraded) => {
                        if let Err(err) = Self::tunnel(upgraded, stream).await {
                            error!("Tunnel failed: {}", err);
                        } else {
                            trace!("Successfully closed the tunnel - no errors")
                        }
                    }
                    Err(err) => error!("Failed to upgrade tunnel connection: {}", err),
                },
                Err(err) => error!("Failed to connect to {host}:{port}: {}", err),
            }
        };

        self.task_pool
            .spawn(main_task)
            .await
            .expect("failed to spawn main tunnel task");

        Ok(Response::new(util::empty()))
    }

    async fn tunnel(upgraded: Upgraded, mut tor_stream: DataStream) -> io::Result<()> {
        let mut client_stream = TokioIo::new(upgraded);

        let (_client_count, _target_count) =
            tokio::io::copy_bidirectional(&mut client_stream, &mut tor_stream).await?;

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

        let (_token, session) = self.fetch_session(util::extract_token(&req)).await;

        trace!("Attempting an unsafe connection to: {}", uri);

        // Connect through Tor owo.
        let mut stream_prefs = self.prefs.stream.clone();

        let stream = match self
            .try_connect_individual((host, port), &mut stream_prefs, session, 10)
            .await
        {
            Ok(s) => {
                let circ = s
                    .client_stream_ctrl()
                    .expect("failed to get client stream ctrl?!")
                    .circuit()
                    .expect("failed to get client circuit?!");

                let target = circ.channel().target();
                if let Some(addr) = target.chan_method().target_addr() {
                    info!("Connecting with target: {:?}", addr);
                } else {
                    info!("Target: {:?}", target)
                }
                TokioIo::new(s)
            }
            Err(err) => {
                warn!("Tor connection failed: {}", err);
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(util::full(format!("Tor connection failed: {}\n", err)))
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
            .body(util::full(body_bytes))
            .unwrap();

        response.headers_mut().extend(headers);

        Ok(response)
    }
}
