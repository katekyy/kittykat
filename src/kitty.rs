use std::{collections::HashMap, sync::Arc, time::Duration};

use arti_client::TorClient;
use bytes::Bytes;
use http::{HeaderValue, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::{
    body::Incoming,
    rt::{Read, Write},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use tokio::sync::Mutex;
use tor_rtcompat::PreferredRuntime;
use tracing::{debug, info, trace, warn};
use uuid::Uuid;

const DEFAULT_UUID: Uuid = Uuid::nil();

type ClientPool = HashMap<Uuid, TorClient<PreferredRuntime>>;

#[derive(Clone, Debug)]
pub struct Preferences {
    pub client_lifetime: Duration,
    pub pool_bound: Option<usize>,
}

#[derive(Clone)]
pub struct KittyKat {
    client_pool: Arc<Mutex<ClientPool>>,
    task_pool: Arc<tokio_task_pool::Pool>,
    preferences: Preferences,
}

impl KittyKat {
    pub fn new(base_client: TorClient<PreferredRuntime>, prefs: Preferences) -> Self {
        info!("Creating a KittyKat instance with preferences: {prefs:?}");

        let mut map = HashMap::new();
        map.insert(DEFAULT_UUID, base_client);

        let task_pool = if let Some(bound) = prefs.pool_bound {
            tokio_task_pool::Pool::bounded(bound * 2)
        } else {
            tokio_task_pool::Pool::unbounded()
        };

        Self {
            client_pool: Arc::new(Mutex::new(map)),
            task_pool: Arc::new(task_pool),
            preferences: prefs,
        }
    }

    pub async fn serve_connection<I>(&self, io: I)
    where
        I: Read + Write + Unpin + Send + 'static,
    {
        let self_clone = self.clone();
        let service = service_fn(move |req| Self::handle_request(self_clone.clone(), req));
        self.task_pool
            .spawn(async move {
                match http1::Builder::new().serve_connection(io, service).await {
                    Ok(()) => {}
                    Err(err) => eprintln!("Got an error while serving a connection: {}", err),
                }
            })
            .await
            .unwrap();
    }

    async fn fork_client(&self) -> (Uuid, TorClient<PreferredRuntime>) {
        let new_id = Uuid::new_v4();

        let client_pool = Arc::clone(&self.client_pool);
        let prefs = self.preferences.clone();

        debug!(
            "Forking a new Tor client {}, purge after {:#?}",
            new_id, prefs.client_lifetime
        );

        self.task_pool
            .spawn(async move {
                tokio::time::sleep(prefs.client_lifetime).await;
                debug!("Purging client {} from the pool", new_id);
                client_pool.lock().await.remove(&new_id);
            })
            .await
            .unwrap();

        let mut client_pool = self.client_pool.lock().await;

        let new_client = client_pool.get(&DEFAULT_UUID).unwrap().isolated_client();
        client_pool.insert(new_id, new_client.clone());

        (new_id, new_client)
    }

    async fn get_or_fork_client(&self, id: &str) -> Option<(Uuid, TorClient<PreferredRuntime>)> {
        if id.is_empty() {
            Some(self.fork_client().await)
        } else {
            match Uuid::parse_str(id) {
                Ok(id) => {
                    let maybe_client = {
                        let client_pool_g = self.client_pool.lock().await;
                        client_pool_g.get(&id).cloned()
                    };
                    if let Some(client) = maybe_client {
                        Some((id, client))
                    } else {
                        Some(self.fork_client().await)
                    }
                }
                Err(_) => None,
            }
        }
    }

    async fn handle_request(
        self,
        req: Request<Incoming>,
    ) -> Result<Response<BoxBody<hyper::body::Bytes, hyper::Error>>, hyper::Error> {
        trace!("Got a new request: {:?}", &req);

        // Validate whether the URI is absolute
        let uri = req.uri().clone();
        if let None = uri.host() {
            debug!(
                "Proxying failed for URI: {:?} - Absolute URI required",
                &uri
            );
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(full("Absolute URI required"))
                .unwrap());
        }

        // Get or fork a Tor client from the pool
        let (id, client) = match self
            .get_or_fork_client(
                req.headers()
                    .get("x-kitty")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
            )
            .await
        {
            Some(pair) => pair,
            None => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(full("Invalid ID"))
                    .unwrap());
            }
        };

        // Parse target details
        let (host, port) = (
            uri.host().expect("Validated absolute URI"),
            uri.port_u16().unwrap_or(80),
        );

        // Connect through Tor >w<
        let stream = match client.connect((host, port)).await {
            Ok(s) => TokioIo::new(s),
            Err(e) => {
                warn!("Tor connection failed: {}", e);
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(full(format!("Tor connection failed: {}", e)))
                    .unwrap());
            }
        };

        // Perform the HTTP handshake
        let (mut sender, conn) = hyper::client::conn::http1::handshake(stream).await?;

        tokio::spawn(async move {
            if let Err(e) = conn.await {
                eprintln!("Connection error: {}", e);
            }
        });

        // Convert request for forwarding
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
        outgoing_req.headers_mut().unwrap().remove("x-kitty"); // Remove our header
        outgoing_req
            .headers_mut()
            .unwrap()
            .insert("host", HeaderValue::from_str(host).unwrap());

        let outgoing_req = outgoing_req.body(body).unwrap();

        // Forward request and relay response
        let mut response = sender.send_request(outgoing_req).await?;
        let status = response.status();
        let version = response.version();

        let mut headers = std::mem::take(response.headers_mut());
        headers.insert("x-kitty", HeaderValue::from_str(&id.to_string()).unwrap());

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
