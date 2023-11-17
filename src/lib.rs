use reqwest::Client;

pub mod handlers;

#[derive(Debug, Clone)]
pub struct AppState {
    pub auth: Option<String>,
    pub client: Client,
    pub registry: Registry,
    pub token_endpoint: String,
}

#[derive(Debug, Clone)]
pub struct Registry {
    pub host: String,
    pub repo_prefix: String,
}