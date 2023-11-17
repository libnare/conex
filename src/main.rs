use std::fs::File;
use std::io::Read;

use actix_web::{App, HttpServer, web};
use base64::{Engine as _, engine::general_purpose};
use regex::Regex;
use reqwest::Client;
use tracing::{error, info, Level};

use conex::{AppState, Registry};
use conex::handlers::config_routes;

async fn discover_token(registry_host: String, client: Client) -> String {
    let url = format!("https://{}/v2/", registry_host);
    let client = client.clone();
    let request = client.get(url.as_str());
    let response = match request.send().await {
        Ok(response) => response,
        Err(e) => {
            error!("Unable to discover the token endpoint of the target registry: {}", e);

            std::process::exit(1);
        }
    };
    let hdr = response.headers().get("www-authenticate").unwrap();
    if hdr == "" {
        error!("'www-authenticate' header not returned from {}, unable to locate the token endpoint", url);
        std::process::exit(1);
    }
    let realm = match Regex::new(r#"realm="(.*)""#).unwrap().captures(hdr.to_str().unwrap()) {
        Some(captures) => captures.get(1).unwrap().as_str().to_owned(),
        None => {
            error!("Unable to locate 'realm' in the 'www-authenticate' response header of {}: {}", url, hdr.to_str().unwrap());
            std::process::exit(1);
        }
    };
    info!("Token endpoint discovered for backend registry: {}", realm);
    realm
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    let host = std::env::var("HOST").unwrap_or("0.0.0.0".to_owned());
    let port = std::env::var("PORT").unwrap_or("8080".to_owned());
    let registry_host = match std::env::var("REGISTRY_HOST") {
        Ok(host) => host,
        Err(_) => {
            error!("REGISTRY_HOST is not set");
            std::process::exit(1);
        }
    };
    let repo_prefix = match std::env::var("REPO_PREFIX") {
        Ok(prefix) => prefix,
        Err(_) => {
            error!("REPO_PREFIX is not set");
            std::process::exit(1);
        }
    };

    let mut auth: Option<String> = None;

    if let Ok(key) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
        let mut file = match File::open(key) {
            Ok(file) => file,
            Err(_) => {
                error!("GOOGLE_APPLICATION_CREDENTIALS is set, but the file cannot be opened.");
                std::process::exit(1);
            }
        };
        let mut contents = String::new();
        file.read_to_string(&mut contents).as_ref().unwrap();
        let base64 = general_purpose::STANDARD.encode(format!("_json_key:{}", contents).as_bytes());
        info!("Google service account authentication is configured.");
        auth = Some(format!("Basic {}", base64));
    } else if let Ok(basic) = std::env::var("AUTH_HEADER") {
        info!("Authentication header is configured.");
        auth = Some(basic);
    }

    let client = Client::new();
    let token_endpoint = discover_token(registry_host.clone(), client.clone()).await;

    let state = AppState {
        auth,
        client,
        registry: Registry {
            host: registry_host,
            repo_prefix,
        },
        token_endpoint,
    };

    let server = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(config_routes)
    })
        .bind(format!("{}:{}", host, port))?;

    info!("Starting server on {}:{}", host, port);
    server.run().await
}