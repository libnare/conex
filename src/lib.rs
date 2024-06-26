use std::env;
use std::fs::File;
use std::io::Read;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use lazy_static::lazy_static;
use reqwest::Client;
use tracing::{error, info};
use url::Url;

pub mod handlers;

lazy_static! {
    static ref CLIENT: Client = {
        Client::new()
    };
}

static PACKAGE_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Debug, Clone)]
pub struct AppState {
    pub auth: Option<String>,
    pub client: Client,
    pub hostname: Option<String>,
    pub registry: Registry,
}

#[derive(Debug, Clone)]
pub struct Registry {
    pub endpoint: Url,
    pub token_endpoint: Url,
    pub repo_prefix: String,
}

#[derive(Debug, Clone)]
pub struct Bind {
    pub host: Option<String>,
    pub port: Option<u16>,
}

impl Default for Bind {
    fn default() -> Self {
        Self {
            host: env::var("BIND_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()).into(),
            port: env::var("BIND_PORT").unwrap_or_else(|_| "8080".to_string()).parse().ok(),
        }
    }
}

impl AppState {
    pub async fn new() -> Self {
        let mut auth: Option<String> = None;
        if let Ok(key) = env::var("GOOGLE_APPLICATION_CREDENTIALS") {
            let mut file = match File::open(key) {
                Ok(file) => file,
                Err(_) => {
                    error!("GOOGLE_APPLICATION_CREDENTIALS is set, but the file cannot be opened.");
                    std::process::exit(1);
                }
            };
            let mut contents = String::new();
            file.read_to_string(&mut contents).as_ref().unwrap();
            let base64 = STANDARD.encode(format!("_json_key:{}", contents).as_bytes());
            info!("Google service account authentication is configured.");
            auth = Some(format!("Basic {}", base64));
        } else if let Ok(basic) = env::var("AUTH_HEADER") {
            info!("Authentication header is configured.");
            auth = Some(basic);
        }
        Self {
            auth,
            hostname: env::var("HOSTNAME").ok(),
            client: CLIENT.clone(),
            registry: Registry::new().await,
        }
    }
}

impl Registry {
    async fn new() -> Self {
        let endpoint = match Url::parse(&env::var("REGISTRY_HOST").unwrap_or_else(|_| "https://index.docker.io".to_string())) {
            Ok(url) => url,
            Err(e) => {
                error!("REGISTRY_HOST is not a valid URL: {}", e);
                std::process::exit(1);
            }
        };
        let token_endpoint = discover_token(endpoint.clone()).await;
        Self {
            endpoint,
            token_endpoint,
            repo_prefix: match env::var("REGISTRY_PREFIX") {
                Ok(prefix) => prefix,
                Err(_) => {
                    error!("REGISTRY_PREFIX is not set");
                    std::process::exit(1);
                }
            },
        }
    }
}

async fn discover_token(registry_host: Url) -> Url {
    let url = format!("{}v2/", registry_host);
    let res = match CLIENT.get(Url::parse(&url).unwrap()).send().await {
        Ok(res) => res,
        Err(e) => {
            error!("Unable to discover the token endpoint of the target registry: {}", e);
            std::process::exit(1);
        }
    };
    let hdr = match res.headers().get("www-authenticate") {
        Some(hdr) => hdr,
        None => {
            error!("'www-authenticate' header is not present, unable to locate the token endpoint");
            std::process::exit(1);
        }
    };

    let realm = match hdr.to_str().unwrap().split(',').find(|s| s.contains("realm")) {
        Some(s) => s.split('=').last().unwrap().replace("\"", ""),
        None => {
            error!("'www-authenticate' header does not contain 'realm' attribute, unable to locate the token endpoint");
            std::process::exit(1);
        }
    };
    info!("Discovered token endpoint: {}", realm);

    Url::parse(&realm).unwrap()
}