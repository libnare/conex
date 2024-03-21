use actix_web::{HttpRequest, HttpResponse, Responder, web};
use actix_web::web::{Bytes, Data, Redirect};
use reqwest::header::HeaderMap;
use actix_web::http::header::HeaderMap as ActixHeaderMap;
use tracing::info;
use url::Url;

use crate::{AppState, PACKAGE_NAME, Registry};

fn rewrite_host_header(headers: &mut ActixHeaderMap, registry: &Registry) {
    headers.insert(
        "host".parse().unwrap(), Url::parse(registry.endpoint.as_str()).unwrap().host_str().unwrap().parse().unwrap(),
    );
}

async fn reverse_proxy(data: Data<AppState>, req: HttpRequest, bytes: Bytes, url: Url, headers: ActixHeaderMap) -> HttpResponse {
    let res = data.client.request(req.method().clone(), url.as_str()).headers(HeaderMap::from(headers)).body(bytes).send().await.unwrap();
    let headers = res.headers().clone();

    let mut http_res = HttpResponse::build(res.status()).body(res.bytes().await.unwrap());
    for (key, value) in headers.iter() {
        http_res.headers_mut().insert(key.clone(), value.clone());
    }

    http_res
}

async fn redirect(data: Data<AppState>, req: HttpRequest) -> impl Responder {
    let url = Url::parse(format!("{}{}{}", data.registry.endpoint, data.registry.repo_prefix, req.path()).as_str()).unwrap();
    Redirect::to(url.to_string()).temporary()
}

async fn api_v2(data: Data<AppState>, req: HttpRequest, bytes: Bytes) -> impl Responder {
    fn rewrite_registry_v2url(registry: Registry, req: &HttpRequest) -> Url {
        let uri = req.uri();
        let mut path = uri.path().to_string();

        if path != "/v2/" {
            path = path.replace("/v2/", format!("v2/{}/", registry.repo_prefix).as_str());
        }

        let url = Url::parse(&format!("{}{}", registry.endpoint, path)).unwrap();
        info!("rewrote url: {} into {}", uri, url);
        url
    }
    let url = rewrite_registry_v2url(data.registry.clone(), &req);
    if req.uri().path() == "/v2/" {
        let hostname = if let Some(hostname) = data.hostname.clone() {
            hostname
        } else {
            req.headers().get("host").unwrap().to_str().unwrap().to_string()
        };
        let local_token = Url::parse(
            format!(
                "{}://{}/{}/token",
                req.connection_info().scheme(),
                hostname,
                PACKAGE_NAME).as_str()
        ).unwrap();
        let mut headers = req.headers().clone();
        rewrite_host_header(&mut headers, &data.registry);

        let mut http_res = reverse_proxy(data, req, bytes, url, headers).await;
        http_res.headers_mut().insert(
            "www-authenticate".parse().unwrap(), format!("realm=\"{}\"", local_token).parse().unwrap(),
        );

        http_res
    } else {
        let mut headers = req.headers().clone();
        rewrite_host_header(&mut headers, &data.registry);
        /*if req.path().starts_with("/v2/") && req.path().contains("/manifests/") {
            /*headers.insert(
                "Accept".parse().unwrap(),
                "application/vnd.docker.distribution.manifest.v2+json".parse().unwrap());*/
            info!("Headers: {}", headers.iter().map(|(k, v)| format!("{}: {}", k, v.to_str().unwrap())).collect::<Vec<String>>().join(", "));
        }*/
        if let Some(auth) = data.auth.clone() {
            headers.insert("Authorization".parse().unwrap(), auth.parse().unwrap());
        }

        reverse_proxy(data, req, bytes, url, headers).await
    }
}

async fn token_proxy(data: Data<AppState>, req: HttpRequest, bytes: Bytes) -> impl Responder {
    let query = req.query_string();
    let scope = match query.split("&").find(|q| q.starts_with("scope=")) {
        Some(scope) => scope,
        None => {
            return HttpResponse::BadRequest().body("scope query parameter is missing");
        }
    };
    let new_scope = scope.replace("repository:", format!("repository:{}/", data.registry.repo_prefix).as_str());
    let mut url = Url::parse(data.registry.token_endpoint.as_str()).unwrap();
    url.set_query(Some(query.replace(scope, new_scope.as_str()).as_str()));
    info!("rewrote token: {} into {}", req.uri(), url);

    let mut headers = req.headers().clone();
    rewrite_host_header(&mut headers, &data.registry);

    reverse_proxy(data, req, bytes, url, headers).await
}

pub fn config_routes(cfg: &mut web::ServiceConfig) {
    cfg
        .service(web::resource("/v2/{tail:.*}").route(web::get().to(api_v2)))
        .service(web::resource(format!("/{}/token", PACKAGE_NAME)).route(web::get().to(token_proxy)))
        .service(web::resource("/{tail:.*}").route(web::get().to(redirect)));
}