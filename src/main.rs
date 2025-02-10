#![feature(local_waker, noop_waker)]

use std::time::Duration;

use arti_client::{TorClient, TorClientConfig};
use hyper_util::rt::TokioIo;
use kitty::{KittyKat, Preferences};
use tokio::net::TcpListener;
use tracing::Level;
mod kitty;

// TODO: Have a TOML or Yaml configuration file

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Setting default subscriber failed");

    let listener = TcpListener::bind("127.0.0.1:8118").await?;

    let config = TorClientConfig::default();
    let client = TorClient::builder()
        .config(config)
        .create_bootstrapped()
        .await?;

    let kitty = KittyKat::new(client, Preferences {
        client_lifetime: Duration::from_secs(10),
        pool_bound: None,
    });

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let io = TokioIo::new(stream);
                kitty.serve_connection(io).await;
            }
            Err(err) => {
                eprintln!("Got an error while accepting a connection: {}", err)
            }
        }
    }
}
