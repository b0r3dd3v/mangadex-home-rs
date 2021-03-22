#![warn(clippy::pedantic, clippy::nursery)]
#![allow(clippy::future_not_send)] // We're end users, so this is ok

use std::env::{self, VarError};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use std::{num::ParseIntError, sync::atomic::Ordering};

use actix_web::rt::{spawn, time, System};
use actix_web::web;
use actix_web::web::Data;
use actix_web::{App, HttpServer};
use log::{error, warn, LevelFilter};
use parking_lot::RwLock;
use rustls::{NoClientAuth, ServerConfig};
use simple_logger::SimpleLogger;
use state::{RwLockServerState, ServerState};
use stop::send_stop;
use thiserror::Error;

mod cache;
mod ping;
mod routes;
mod state;
mod stop;

#[macro_export]
macro_rules! client_api_version {
    () => {
        "30"
    };
}
#[derive(Error, Debug)]
enum ServerError {
    #[error("There was a failure parsing config")]
    Config(#[from] VarError),
    #[error("Failed to parse an int")]
    ParseInt(#[from] ParseIntError),
}

#[actix_web::main]
async fn main() -> Result<(), std::io::Error> {
    // It's ok to fail early here, it would imply we have a invalid config.
    dotenv::dotenv().ok();
    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .init()
        .unwrap();
    let config = Config::new().unwrap();
    let port = config.port;
    let server = ServerState::init(&config).await.unwrap();

    // Set ctrl+c to send a stop message
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let client_secret = config.secret.clone();
    ctrlc::set_handler(move || {
        let client_secret = client_secret.clone();
        System::new().block_on(async move {
            send_stop(&client_secret).await;
        });
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    let data_0 = Arc::new(RwLockServerState(RwLock::new(server)));
    let data_1 = Arc::clone(&data_0);
    let data_2 = Arc::clone(&data_0);

    spawn(async move {
        let mut interval = time::interval(Duration::from_secs(90));
        let mut data = Arc::clone(&data_0);
        loop {
            interval.tick().await;
            ping::update_server_state(&config, &mut data).await;
        }
    });

    let mut tls_config = ServerConfig::new(NoClientAuth::new());
    tls_config.cert_resolver = data_2;

    HttpServer::new(move || {
        App::new()
            .service(routes::token_data)
            .service(routes::no_token_data)
            .service(routes::token_data_saver)
            .service(routes::no_token_data_saver)
            .route("{tail:.*}", web::get().to(routes::default))
            .app_data(Data::from(Arc::clone(&data_1)))
    })
    .shutdown_timeout(60)
    .bind_rustls(format!("0.0.0.0:{}", port), tls_config)?
    .run()
    .await?;

    // Waiting for us to finish sending stop message
    while running.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(250));
    }

    Ok(())
}

pub struct Config {
    secret: String,
    port: u16,
    memory_quota: usize,
    disk_quota: usize,
    disk_path: PathBuf,
    network_speed: usize,
}

impl Config {
    fn new() -> Result<Self, ServerError> {
        let secret = env::var("CLIENT_SECRET")?;
        let port = env::var("PORT")?.parse()?;
        let disk_quota = env::var("DISK_CACHE_QUOTA_BYTES")?.parse()?;
        let memory_quota = env::var("MEM_CACHE_QUOTA_BYTES")?.parse()?;
        let network_speed = env::var("MAX_NETWORK_SPEED")?.parse()?;
        let disk_path = env::var("DISK_CACHE_PATH")
            .unwrap_or("./cache".to_string())
            .parse()
            .unwrap();

        Ok(Self {
            secret,
            port,
            disk_quota,
            memory_quota,
            disk_path,
            network_speed,
        })
    }
}
