use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http::{Method, Request, Response, StatusCode, Uri};
use http::header::{HeaderValue, HOST, AUTHORIZATION};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::Service;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tracing::info;
use url::Url;

use crate::{AppState, PACKAGE_NAME};

type BoxBody = http_body_util::combinators::BoxBody<Bytes, std::io::Error>;
type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone)]
pub struct ProxyService {
    state: Arc<AppState>,
    client: Client<hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>, BoxBody>,
}

impl ProxyService {
    pub fn new(state: Arc<AppState>) -> Self {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .build();
        
        let client = Client::builder(TokioExecutor::new()).build(https);
        
        Self { state, client }
    }

    fn rewrite_registry_v2_url(&self, uri: &Uri) -> Url {
        let path = uri.path();
        let registry = &self.state.registry;
        
        let new_path = if path == "/v2/" {
            path.to_string()
        } else {
            path.replace("/v2/", &format!("/v2/{}/", registry.repo_prefix))
        };

        let mut url = registry.endpoint.clone();
        url.set_path(&new_path);
        
        if let Some(query) = uri.query() {
            url.set_query(Some(query));
        }
        
        info!("rewrote url: {} into {}", uri, url);
        url
    }

    fn rewrite_token_scope(&self, query: &str) -> String {
        let parts: Vec<&str> = query.split('&').collect();
        let mut new_parts = Vec::new();
        
        for part in parts {
            if part.starts_with("scope=") {
                let new_scope = part.replace("repository:", &format!("repository:{}/", self.state.registry.repo_prefix));
                new_parts.push(new_scope);
            } else {
                new_parts.push(part.to_string());
            }
        }
        
        new_parts.join("&")
    }

    async fn proxy_request(&self, req: Request<Incoming>) -> Result<Response<BoxBody>, BoxError> {
        let uri = req.uri().clone();
        let path = uri.path();
        
        if path.starts_with("/v2/") {
            self.handle_registry_api(req).await
        } else if path == format!("/{}/token", PACKAGE_NAME) {
            self.handle_token_proxy(req).await
        } else {
            self.handle_redirect(&uri)
        }
    }

    async fn handle_registry_api(&self, req: Request<Incoming>) -> Result<Response<BoxBody>, BoxError> {
        let uri = req.uri().clone();
        let method = req.method().clone();
        let url = self.rewrite_registry_v2_url(&uri);
        
        let mut headers = req.headers().clone();
        let original_host_header = headers.get(HOST).cloned();
        
        if let Ok(host_value) = HeaderValue::from_str(url.host_str().unwrap_or("")) {
            headers.insert(HOST, host_value);
        }
        
        if let Some(auth) = &self.state.auth {
            if let Ok(auth_value) = HeaderValue::from_str(auth) {
                headers.insert(AUTHORIZATION, auth_value);
            }
        }
        
        let body = req.into_body().collect().await.map_err(|e| Box::new(e) as BoxError)?.to_bytes();
        let body = Full::new(body).map_err(|e: std::convert::Infallible| match e {}).boxed();
        
        let new_uri = Uri::try_from(url.as_str()).map_err(|e| Box::new(e) as BoxError)?;
        let mut client_req = Request::builder()
            .method(method)
            .uri(new_uri);
        
        for (key, value) in headers.iter() {
            client_req = client_req.header(key, value);
        }
        
        let client_req = client_req.body(body).map_err(|e| Box::new(e) as BoxError)?;
        
        let client_resp = self.client.request(client_req).await
            .map_err(|e| {
                tracing::error!("Failed to execute request: {}", e);
                Box::new(e) as BoxError
            })?;
        
        let status = client_resp.status();
        let mut response = Response::builder().status(status);
        
        for (key, value) in client_resp.headers() {
            response = response.header(key.as_str(), value.as_bytes());
        }
        
        if uri.path() == "/v2/" {
            let hostname = self.state.hostname.clone()
                .or_else(|| original_host_header.and_then(|h| h.to_str().ok().map(String::from)))
                .unwrap_or_else(|| "localhost".to_string());
                
            let scheme = if hostname.starts_with("localhost") { "http" } else { "https" };
            let local_token = format!("{}://{}/{}/token", scheme, hostname, PACKAGE_NAME);
            
            response = response.header("www-authenticate", format!("Bearer realm=\"{}\"", local_token));
        }
        
        let body = client_resp.into_body().map_err(std::io::Error::other).boxed();
        response.body(body).map_err(|e| {
            tracing::error!("Failed to build response: {}", e);
            Box::new(e) as BoxError
        })
    }

    async fn handle_token_proxy(&self, req: Request<Incoming>) -> Result<Response<BoxBody>, BoxError> {
        let query = req.uri().query().unwrap_or("");
        let new_query = self.rewrite_token_scope(query);
        
        let mut url = self.state.registry.token_endpoint.clone();
        url.set_query(Some(&new_query));
        
        info!("rewrote token: {} into {}", req.uri(), url);
        
        let mut headers = req.headers().clone();
        if let Ok(host_value) = HeaderValue::from_str(url.host_str().unwrap_or("")) {
            headers.insert(HOST, host_value);
        }
        
        let body = req.into_body().collect().await.map_err(|e| Box::new(e) as BoxError)?.to_bytes();
        let body = Full::new(body).map_err(|e: std::convert::Infallible| match e {}).boxed();
        
        let new_uri = Uri::try_from(url.as_str()).map_err(|e| Box::new(e) as BoxError)?;
        let mut client_req = Request::builder()
            .method(Method::GET)
            .uri(new_uri);
        
        for (key, value) in headers.iter() {
            client_req = client_req.header(key, value);
        }
        
        let client_req = client_req.body(body).map_err(|e| Box::new(e) as BoxError)?;
        
        let client_resp = self.client.request(client_req).await
            .map_err(|e| {
                tracing::error!("Failed to execute token request: {}", e);
                Box::new(e) as BoxError
            })?;
        
        let status = client_resp.status();
        let mut response = Response::builder().status(status);
        
        for (key, value) in client_resp.headers() {
            response = response.header(key.as_str(), value.as_bytes());
        }
        
        let body = client_resp.into_body().map_err(std::io::Error::other).boxed();
        response.body(body).map_err(|e| {
            tracing::error!("Failed to build token response: {}", e);
            Box::new(e) as BoxError
        })
    }

    fn handle_redirect(&self, uri: &Uri) -> Result<Response<BoxBody>, BoxError> {
        let path = uri.path();
        let registry = &self.state.registry;
        
        let redirect_url = format!("{}{}{}", 
            registry.endpoint,
            registry.repo_prefix,
            path
        );
        
        let body = Full::new(Bytes::from("Redirecting...")).map_err(|e: std::convert::Infallible| match e {}).boxed();
        
        Response::builder()
            .status(StatusCode::TEMPORARY_REDIRECT)
            .header("Location", redirect_url)
            .body(body)
            .map_err(|e| {
                tracing::error!("Failed to build redirect response: {}", e);
                Box::new(e) as BoxError
            })
    }
}

impl Service<Request<Incoming>> for ProxyService {
    type Response = Response<BoxBody>;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        let service = self.clone();
        Box::pin(async move {
            service.proxy_request(req).await
        })
    }
}