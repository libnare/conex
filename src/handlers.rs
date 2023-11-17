use actix_web::{HttpRequest, HttpResponse, HttpResponseBuilder, Responder, web};
use actix_web::web::{Bytes, Data, Redirect};
use regex::Regex;
use reqwest::header::HeaderMap;
use reqwest::Response;
use tracing::info;
use url::Url;

use crate::AppState;

fn clone_req_headers(req: &HttpRequest, data: Data<AppState>, auth: bool) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (key, value) in req.headers().iter() {
        headers.insert(key.clone(), value.clone());
    }
    if auth {
        let basic = data.auth.clone().unwrap();
        headers.insert("Authorization", basic.parse().unwrap());
    }
    headers.insert("Host", data.registry.host.parse().unwrap());
    if req.path().starts_with("/v2/") && req.path().contains("/manifests/") {
        headers.insert("Accept", "application/vnd.docker.distribution.manifest.v2+json".parse().unwrap());
    }
    headers
}

fn clone_res_headers(req: &HttpRequest, res: &Response, builder: &mut HttpResponseBuilder, realm: bool) {
    let local_token = Url::parse(format!("https://{}/conex/token", req.headers().get("host").unwrap().to_str().unwrap()).as_str()).unwrap();
    for (key, value) in res.headers().iter() {
        if key == "Content-Length" {
            continue;
        }
        builder.insert_header((key.clone(), value.clone()));
    }
    if realm {
        builder.insert_header(("www-authenticate", format!("realm=\"{}\"", local_token)));
    }
}

async fn rewrite_registry_v2url(data: Data<AppState>, req: HttpRequest) -> Url {
    let uri = req.uri();
    let host = data.registry.host.clone();
    let scheme = "https".to_owned();
    let mut path = req.path().to_owned();

    if path != "/v2/" {
        path = Regex::new(r"^/v2/").unwrap().replace(&path, format!("/v2/{}/", data.registry.repo_prefix).as_str()).to_string();
    }

    let url = Url::parse(format!("{}://{}{}", scheme, host, path).as_str()).unwrap();
    info!("rewrote url: {} into {}", uri, url);
    url
}

async fn api_v2(data: Data<AppState>, req: HttpRequest, bytes: Bytes) -> impl Responder {
    let url = rewrite_registry_v2url(data.clone(), req.clone()).await;
    let auth = if let Some(_) = data.auth.clone() {
        true
    } else {
        false
    };
    if req.path() == "/v2/" {
        reverse_proxy(data.clone(), url, req, bytes, true, false).await
    } else {
        reverse_proxy(data.clone(), url, req, bytes, false, auth).await
    }
}

async fn reverse_proxy(data: Data<AppState>, url: Url, req: HttpRequest, bytes: Bytes, realm: bool, auth: bool) -> HttpResponse {
    let client = data.client.clone();
    let body = bytes.to_vec();
    let request = client.request(req.method().clone(), url.as_str());
    let request = request.body(body).headers(clone_req_headers(&req, data, auth));
    let response = request.send().await.unwrap();
    let mut http_response = HttpResponse::build(response.status());
    clone_res_headers(&req, &response, &mut http_response, realm);
    let body = response.bytes().await.unwrap();
    http_response.body(body)
}

async fn token_proxy(data: Data<AppState>, req: HttpRequest, bytes: Bytes) -> impl Responder {
    let query = req.query_string();
    let scope = match query.split("&").find(|q| q.starts_with("scope=")) {
        Some(scope) => scope,
        None => {
            return HttpResponse::BadRequest().body("scope is not set");
        }
    };
    let new_scope = scope.replace("repository:", format!("repository:{}/", data.registry.repo_prefix).as_str());
    let mut url = Url::parse(data.token_endpoint.as_str()).unwrap();
    url.set_query(Some(query.replace(scope, new_scope.as_str()).as_str()));
    info!("rewrote token: {} into {}", req.uri(), url);
    reverse_proxy(data, url, req, bytes, false, false).await
}

async fn redirect(data: Data<AppState>, req: HttpRequest) -> impl Responder {
    let url = Url::parse(format!("https://{}/{}{}", data.registry.host, data.registry.repo_prefix, req.path()).as_str()).unwrap();
    Redirect::to(url.to_string()).temporary()
}

pub fn config_routes(cfg: &mut web::ServiceConfig) {
    cfg
        .service(web::resource("/").route(web::get().to(redirect)))
        .service(web::resource("/v2/{tail:.*}").route(web::get().to(api_v2)))
        .service(web::resource("/conex/token").route(web::get().to(token_proxy)));
}