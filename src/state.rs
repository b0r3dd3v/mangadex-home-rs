use std::{io::BufReader, sync::Arc};

use crate::cache::Cache;
use crate::ping::{Request, Response, Tls, CONTROL_CENTER_PING_URL};
use crate::Config;
use log::{debug, error, info, warn};
use parking_lot::RwLock;
use rustls::internal::pemfile::{certs, rsa_private_keys};
use rustls::sign::{CertifiedKey, RSASigningKey};
use rustls::ResolvesServerCert;
use sodiumoxide::crypto::box_::PrecomputedKey;
use url::Url;

pub struct ServerState {
    pub precomputed_key: PrecomputedKey,
    pub image_server: Url,
    pub tls_config: Tls,
    pub force_tokens: bool,
    pub url: String,
    pub cache: Cache,
}

impl ServerState {
    pub async fn init(config: &Config) -> Result<Self, ()> {
        let resp = reqwest::Client::new()
            .post(CONTROL_CENTER_PING_URL)
            .json(&Request::from(config))
            .send()
            .await;

        match resp {
            Ok(resp) => match resp.json::<Response>().await {
                Ok(resp) => {
                    let key = resp
                        .token_key
                        .and_then(|key| {
                            if let Some(key) = base64::decode(&key)
                                .ok()
                                .and_then(|k| PrecomputedKey::from_slice(&k))
                            {
                                Some(key)
                            } else {
                                error!("Failed to parse token key: got {}", key);
                                None
                            }
                        })
                        .unwrap();

                    if resp.compromised {
                        warn!("Got compromised response from control center!");
                    }

                    if resp.paused {
                        debug!("Got paused response from control center.");
                    }

                    info!("This client's URL has been set to {}", resp.url);

                    if resp.force_tokens {
                        info!("This client will validate tokens");
                    }

                    Ok(Self {
                        precomputed_key: key,
                        image_server: resp.image_server,
                        tls_config: resp.tls.unwrap(),
                        force_tokens: resp.force_tokens,
                        url: resp.url,
                        cache: Cache::new(
                            config.memory_quota,
                            config.disk_quota,
                            config.disk_path.clone(),
                        ),
                    })
                }
                Err(e) => {
                    warn!("Got malformed response: {}", e);
                    Err(())
                }
            },
            Err(e) => match e {
                e if e.is_timeout() => {
                    error!("Response timed out to control server. Is MangaDex down?");
                    Err(())
                }
                e => {
                    warn!("Failed to send request: {}", e);
                    Err(())
                }
            },
        }
    }
}

pub struct RwLockServerState(pub RwLock<ServerState>);

impl ResolvesServerCert for RwLockServerState {
    fn resolve(&self, _: rustls::ClientHello) -> Option<CertifiedKey> {
        let read_guard = self.0.read();
        let priv_key = rsa_private_keys(&mut BufReader::new(
            read_guard.tls_config.private_key.as_bytes(),
        ))
        .ok()?
        .pop()
        .unwrap();

        let certs = certs(&mut BufReader::new(
            read_guard.tls_config.certificate.as_bytes(),
        ))
        .ok()?;

        Some(CertifiedKey {
            cert: certs,
            key: Arc::new(Box::new(RSASigningKey::new(&priv_key).unwrap())),
            ocsp: None,
            sct_list: None,
        })
    }
}