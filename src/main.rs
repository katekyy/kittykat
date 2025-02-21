use std::{collections::HashMap, env, sync::Arc};

use anyhow::anyhow;
use arti_client::{StreamPrefs, TorClient, TorClientConfig};
use config::{Config, CONFIG_FILE};
use hyper_util::rt::TokioIo;
use kitty::{KittyKat, Preferences};
use tokio::{fs::File, io::AsyncReadExt, net::TcpListener};
use tracing::info;

mod config;
mod kitty;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // We get the configuration from either the environment or a file.
    // Environment configuration takes precendece over a file config.
    // Both sources are not merged, we take it from one that works.

    let vars = env::vars().collect::<HashMap<String, String>>();

    let config = match Config::from_env(&vars)
        .inspect_err(|err| eprintln!("Failed to acquire configuration from environment: {err}"))
    {
        Ok(config) => config,
        Err(config::Error::ParseError(msg)) => return Err(anyhow!(msg)),
        Err(config::Error::MissingEnvironmentVariable(_)) => {
            let mut config_file = File::open(CONFIG_FILE).await.map_err(|err| {
                anyhow!(
                    "Could not open {} in the current working directory: {}",
                    CONFIG_FILE,
                    err
                )
            })?;

            let mut config_buf = String::with_capacity(512);
            config_file.read_to_string(&mut config_buf).await?;

            match Config::from_toml(config_buf) {
                Ok(config) => config,
                Err(err) => return Err(anyhow!(err)),
            }
        }
    };

    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(config.log_level())
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Setting default subscriber failed");

    info!("Starting a listener on {}", config.listen_address);

    let listener = TcpListener::bind(config.listen_address).await?;

    let mut client_config = TorClientConfig::builder();

    client_config
        .circuit_timing()
        .max_dirtiness(config.max_circuit_dirtiness());

    let client_config = client_config.build()?;

    let client = TorClient::with_runtime(tor_rtcompat::tokio::PreferredRuntime::current().unwrap())
        .config(client_config)
        .create_bootstrapped()
        .await?;

    let mut prefs = StreamPrefs::new();

    if config.optimistic_stream {
        prefs.optimistic();
    }

    let kitty = KittyKat::new(
        client,
        Preferences {
            token_lifetime: config.token_lifetime(),
            pool_bound: None, // TODO: acknowledge in configuration
            stream_prefs: prefs,
            ..Default::default()
        },
    )
    .await;

    let kitty = Arc::new(kitty);

    loop {
        let kitty = Arc::clone(&kitty);
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let io = TokioIo::new(stream);
                tokio::spawn(async move { kitty.serve_connection(io).await });
            }
            Err(err) => {
                eprintln!("Got an error while accepting a connection: {}", err)
            }
        }
    }
}
