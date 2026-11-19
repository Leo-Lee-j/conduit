use super::*;
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, Request as HttpRequest, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, TcpListener, UdpSocket},
    sync::{Mutex, MutexGuard},
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::sync::{Semaphore, watch};

const PAIR_LIFETIME: u64 = 300;
const DEVICE_LIFETIME: u64 = 12 * 3600;
const REQUEST_LIFETIME: u64 = 600;
const MAX_OPERATIONS: usize = 256;

#[derive(Clone)]
pub(crate) struct Host(Arc<Shared>);

struct Shared {
    origin: String,
    authority: String,
    cookie: &'static str,
    secure: bool,
    epoch: String,
    fingerprint_key: String,
    identity: Mutex<Identity>,
    requests: async_channel::Sender<Request>,
    revision: watch::Sender<u64>,
    shutdown: watch::Sender<bool>,
    api_slots: Arc<Semaphore>,
    event_slots: Arc<Semaphore>,
}

struct Identity {
    pair: Option<(String, u64)>,
    next_pair_attempt: Option<std::time::Instant>,
    device: Option<Device>,
    operations: HashMap<String, Operation>,
}

#[derive(Clone)]
struct Device {
    id: Arc<str>,
    cookie_hash: [u8; 32],
    csrf: String,
    name: String,
    expires: u64,
}

struct Operation {
    fingerprint: [u8; 32],
    expires: u64,
    created: u64,
    updated: u64,
    state: &'static str,
    result: Option<Value>,
    error: Option<Fault>,
}

impl Host {
    pub(crate) fn start(origin: &str) -> std::io::Result<(Self, async_channel::Receiver<Request>)> {
        Self::bind(origin, PORT)
    }

    pub(crate) fn start_lan() -> std::io::Result<(Self, async_channel::Receiver<Request>)> {
        let address = lan_ipv4()?;
        Self::bind_lan(address, PORT)
    }

    fn bind_lan(
        address: Ipv4Addr,
        port: u16,
    ) -> std::io::Result<(Self, async_channel::Receiver<Request>)> {
        if !address.is_private() {
            return Err(std::io::Error::other("Choose a private IPv4 address."));
        }
        let listener = TcpListener::bind((address, port))?;
        let origin = format!("http://{address}:{}", listener.local_addr()?.port());
        Self::launch(listener, &origin, false)
    }

    fn bind(origin: &str, port: u16) -> std::io::Result<(Self, async_channel::Receiver<Request>)> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        let local_origin = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
        let origin = if origin.is_empty() {
            &local_origin
        } else {
            origin
        };
        let url = url::Url::parse(origin)
            .map_err(|_| std::io::Error::other("Enter a Tailscale HTTPS origin."))?;
        let secure = url.scheme() == "https"
            && url
                .domain()
                .is_some_and(|domain| domain.ends_with(".ts.net"));
        let normalized = url.origin().ascii_serialization();
        if (!secure && normalized != local_origin) || origin.trim_end_matches('/') != normalized {
            return Err(std::io::Error::other(
                "Use the exact Tailscale Serve HTTPS address or leave it empty for local preview.",
            ));
        }
        Self::launch(listener, &normalized, secure)
    }

    fn launch(
        listener: TcpListener,
        origin: &str,
        secure: bool,
    ) -> std::io::Result<(Self, async_channel::Receiver<Request>)> {
        listener.set_nonblocking(true)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (requests, receiver) = async_channel::bounded(32);
        let (revision, _) = watch::channel(0);
        let (shutdown, _) = watch::channel(false);
        let host = Self(Arc::new(Shared {
            authority: origin.split_once("://").unwrap().1.into(),
            origin: origin.to_owned(),
            cookie: if secure {
                "__Host-conduit_remote"
            } else {
                "conduit_remote_preview"
            },
            secure,
            epoch: secret()?,
            fingerprint_key: secret()?,
            identity: Mutex::new(Identity {
                pair: None,
                next_pair_attempt: None,
                device: None,
                operations: HashMap::new(),
            }),
            requests,
            revision,
            shutdown,
            api_slots: Arc::new(Semaphore::new(32)),
            event_slots: Arc::new(Semaphore::new(4)),
        }));
        let service = host.clone();
        std::thread::Builder::new()
            .name("conduit-remote".into())
            .spawn(move || {
                runtime.block_on(async {
                    if let Ok(listener) = tokio::net::TcpListener::from_std(listener) {
                        let mut shutdown = service.0.shutdown.subscribe();
                        let server = std::future::IntoFuture::into_future(axum::serve(
                            listener,
                            router(service.clone()),
                        ));
                        // Dropping the runtime also closes idle sockets and any stalled response.
                        let _ = futures_util::future::select(
                            Box::pin(server),
                            Box::pin(shutdown.wait_for(|stop| *stop)),
                        )
                        .await;
                    }
                    service.stop();
                });
            })?;
        Ok((host, receiver))
    }

    fn identity(&self) -> MutexGuard<'_, Identity> {
        self.0
            .identity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn origin(&self) -> &str {
        &self.0.origin
    }
    pub(crate) fn epoch(&self) -> &str {
        &self.0.epoch
    }
    pub(crate) fn active(&self) -> bool {
        !*self.0.shutdown.borrow()
    }

    pub(crate) fn stop(&self) {
        self.0.shutdown.send_replace(true);
        self.0.requests.close();
        let mut identity = self.identity();
        identity.device = None;
        identity.pair = None;
        drop(identity);
        self.changed();
    }

    pub(crate) fn link(&self) -> std::io::Result<String> {
        let secret = secret()?;
        self.identity().pair = Some((secret.clone(), now() + PAIR_LIFETIME));
        Ok(format!("{}/#pair={secret}", self.origin()))
    }

    pub(crate) fn changed(&self) {
        self.0
            .revision
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    pub(crate) fn authorized(&self, device: &str) -> bool {
        self.active()
            && self
                .identity()
                .device
                .as_ref()
                .is_some_and(|current| current.id.as_ref() == device && current.expires > now())
    }

    pub(crate) fn can_apply(&self, device: &str, operation: &str) -> bool {
        let identity = self.identity();
        self.active()
            && identity
                .device
                .as_ref()
                .is_some_and(|current| current.id.as_ref() == device && current.expires > now())
            && identity
                .operations
                .get(operation)
                .is_some_and(|op| op.expires > now() && op.state == "queued")
    }

    pub(crate) fn saving(&self, id: &str, result: Value) {
        self.update(id, "saving", Some(result), None);
    }

    pub(crate) fn complete(&self, id: &str, reply: Reply) {
        match reply {
            Ok(result) => self.update(id, "succeeded", Some(result), None),
            Err(error) => self.update(id, "failed", None, Some(error)),
        }
    }

    fn update(&self, id: &str, state: &'static str, result: Option<Value>, error: Option<Fault>) {
        if let Some(op) = self.identity().operations.get_mut(id) {
            op.state = state;
            op.result = result;
            op.error = error;
            op.updated = now();
        }
        self.changed();
    }
}

fn lan_ipv4() -> std::io::Result<Ipv4Addr> {
    let route = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    // UDP connect selects the system route without sending a packet.
    route
        .connect((Ipv4Addr::new(192, 0, 2, 1), 9))
        .map_err(|_| {
            std::io::Error::other(
                "Connect this computer to a private Wi-Fi or Ethernet network, then try again.",
            )
        })?;
    match route.local_addr()?.ip() {
        std::net::IpAddr::V4(address) if address.is_private() => Ok(address),
        _ => Err(std::io::Error::other(
            "Connect this computer to a private Wi-Fi or Ethernet network, then try again.",
        )),
    }
}

impl IntoResponse for Fault {
    fn into_response(self) -> Response {
        (StatusCode::from_u16(self.http_status).unwrap(), Json(self)).into_response()
    }
}

fn unauthorized() -> Fault {
    Fault::new(401, "unauthorized", "Pair this browser from the desktop.")
}
fn invalid() -> Fault {
    Fault::new(
        400,
        "invalid_request",
        "Check the request fields and try again.",
    )
}
fn forbidden() -> Fault {
    Fault::new(
        403,
        "forbidden",
        "This request must come from the paired page.",
    )
}
fn limit() -> Fault {
    Fault::new(
        429,
        "rate_limited",
        "Too many requests. Try again in a moment.",
    )
}

fn router(host: Host) -> Router {
    let api = Router::new()
        .route("/auth/pair", post(pair))
        .route("/auth/session", get(auth_session))
        .route("/auth/logout", post(logout))
        .route("/system", get(system))
        .route("/workspaces", get(workspaces))
        .route("/workspaces/{workspace}/sessions", get(sessions))
        .route("/sessions", post(create))
        .route("/sessions/{session}", get(session))
        .route(
            "/sessions/{session}/metadata",
            get(metadata).patch(patch_metadata),
        )
        .route("/sessions/{session}/messages", get(messages))
        .route("/sessions/{session}/approvals", get(approvals))
        .route("/sessions/{session}/runs", post(send))
        .route("/sessions/{session}/runs/{run}/cancel", post(cancel))
        .route(
            "/sessions/{session}/runs/{run}/approvals/{ticket}",
            post(approval),
        )
        .route("/operations/{id}", get(operation))
        .route("/events", get(events))
        .layer(DefaultBodyLimit::max(MAX_TEXT * 6 + 1024))
        .layer(middleware::from_fn_with_state(host.clone(), bounded_api));
    Router::new()
        .route(
            "/",
            get(|| async { axum::response::Html(include_str!("../../web/remote/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../../web/remote/app.js"),
                )
            }),
        )
        .route(
            "/app.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../../web/remote/app.css"),
                )
            }),
        )
        .route(
            "/manifest.webmanifest",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "application/manifest+json")],
                    include_str!("../../web/remote/manifest.webmanifest"),
                )
            }),
        )
        .route(
            "/icon.png",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/png")],
                    &include_bytes!("../../assets/app-icons/Square150x150Logo.png")[..],
                )
            }),
        )
        .nest("/api/remote/v1", api)
        .fallback(|| async { Fault::missing() })
        .layer(middleware::from_fn_with_state(host.clone(), same_origin))
        .with_state(host)
}

fn header_text<'a>(headers: &'a HeaderMap, key: &str) -> Option<&'a str> {
    headers.get(key)?.to_str().ok()
}

async fn same_origin(State(host): State<Host>, request: HttpRequest, next: Next) -> Response {
    let headers = request.headers();
    let authority = header_text(headers, "host");
    let is_funnel = headers.contains_key("tailscale-funnel-request");
    let identity_verified = !host.0.secure
        || header_text(headers, "tailscale-user-login").is_some_and(|login| !login.is_empty());
    let same_authority = authority == Some(host.0.authority.as_str());
    let local_authority = host.0.secure
        && authority == Some(format!("127.0.0.1:{PORT}").as_str())
        && header_text(headers, "x-forwarded-host") == Some(host.0.authority.as_str())
        && header_text(headers, "x-forwarded-proto") == Some("https");
    let safe = matches!(*request.method(), Method::GET | Method::HEAD);
    let origin = header_text(headers, "origin");
    if !host.active()
        || is_funnel
        || !identity_verified
        || (!same_authority && !local_authority)
        || (origin.is_some() && origin != Some(host.origin()))
        || (!safe && origin != Some(host.origin()))
        || (request.uri().path().starts_with("/api/")
            && header_text(headers, "sec-fetch-site")
                .is_some_and(|site| site != "same-origin" && site != "none"))
    {
        return forbidden().into_response();
    }
    let mut response = next.run(request).await;
    for (key, value) in [
        ("cache-control", "no-store"),
        (
            "content-security-policy",
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; manifest-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'",
        ),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-resource-policy", "same-origin"),
    ] {
        response
            .headers_mut()
            .insert(key, HeaderValue::from_static(value));
    }
    response
}

async fn bounded_api(State(host): State<Host>, mut request: HttpRequest, next: Next) -> Response {
    let Ok(_permit) = host.0.api_slots.clone().try_acquire_owned() else {
        return limit().into_response();
    };
    // Bind body reads as well as dispatch so slow clients cannot occupy slots indefinitely.
    let result = tokio::time::timeout(Duration::from_secs(8), async {
        if !matches!(*request.method(), Method::GET | Method::HEAD) {
            let body = std::mem::replace(request.body_mut(), Body::empty());
            match axum::body::to_bytes(body, MAX_TEXT * 6 + 1024).await {
                Ok(bytes) => *request.body_mut() = Body::from(bytes),
                Err(_) => {
                    return Fault::new(413, "body_too_large", "Request exceeds the allowed size.")
                        .into_response();
                }
            }
        }
        next.run(request).await
    })
    .await;
    result.unwrap_or_else(|_| Fault::unavailable().into_response())
}

fn auth(host: &Host, headers: &HeaderMap) -> Result<Device, Fault> {
    let mut cookies = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|cookie| cookie.trim().split_once('='))
        .filter(|(name, _)| *name == host.0.cookie)
        .map(|(_, value)| value);
    let token = cookies
        .next()
        .filter(|value| value.len() == 43)
        .ok_or_else(unauthorized)?;
    if cookies.next().is_some() {
        return Err(unauthorized());
    }
    let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    host.identity()
        .device
        .as_ref()
        .filter(|device| device.expires > now() && bool::from(hash.ct_eq(&device.cookie_hash)))
        .cloned()
        .ok_or_else(unauthorized)
}

fn check_csrf(device: &Device, headers: &HeaderMap) -> Result<(), Fault> {
    if header_text(headers, "x-csrf-token")
        .is_some_and(|token| bool::from(token.as_bytes().ct_eq(device.csrf.as_bytes())))
    {
        Ok(())
    } else {
        Err(forbidden())
    }
}

fn session_json(device: &Device) -> Value {
    json!({"device_id": device.id.as_ref(), "device_name": device.name, "csrf_token": device.csrf,
        "server_time": now(), "expires_at": device.expires})
}

fn cookie(host: &Host, value: &str, max_age: u64) -> String {
    let secure = if host.0.secure { "; Secure" } else { "" };
    format!(
        "{}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}",
        host.0.cookie
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pair {
    secret: String,
    device_name: String,
}

async fn pair(
    State(host): State<Host>,
    Json(pair): Json<Pair>,
) -> Result<impl IntoResponse, Fault> {
    let name = pair.device_name.trim();
    if name.is_empty() || name.chars().count() > 80 || pair.secret.len() != 43 {
        return Err(invalid());
    }
    let mut identity = host.identity();
    let instant = std::time::Instant::now();
    if identity
        .next_pair_attempt
        .is_some_and(|next| next > instant)
    {
        return Err(limit());
    }
    identity.next_pair_attempt = Some(instant + Duration::from_millis(400));
    if !host.active()
        || !identity.pair.as_ref().is_some_and(|(secret, expires)| {
            *expires > now() && bool::from(secret.as_bytes().ct_eq(pair.secret.as_bytes()))
        })
    {
        return Err(unauthorized());
    }
    let token = secret().map_err(|_| Fault::unavailable())?;
    let device = Device {
        id: Arc::from(secret().map_err(|_| Fault::unavailable())?),
        cookie_hash: Sha256::digest(token.as_bytes()).into(),
        csrf: secret().map_err(|_| Fault::unavailable())?,
        name: name.into(),
        expires: now() + DEVICE_LIFETIME,
    };
    identity.pair = None;
    identity.device = Some(device.clone());
    drop(identity);
    host.changed();
    Ok((
        [(header::SET_COOKIE, cookie(&host, &token, DEVICE_LIFETIME))],
        Json(session_json(&device)),
    ))
}

async fn auth_session(State(host): State<Host>, headers: HeaderMap) -> Result<Json<Value>, Fault> {
    Ok(Json(session_json(&auth(&host, &headers)?)))
}

async fn logout(State(host): State<Host>, headers: HeaderMap) -> Result<impl IntoResponse, Fault> {
    let device = auth(&host, &headers)?;
    check_csrf(&device, &headers)?;
    let mut identity = host.identity();
    if identity
        .device
        .as_ref()
        .is_some_and(|current| current.id == device.id)
    {
        identity.device = None;
    }
    drop(identity);
    host.changed();
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, cookie(&host, "", 0))],
    ))
}

async fn read(host: Host, headers: HeaderMap, query: Read) -> Result<Response, Fault> {
    let device = auth(&host, &headers)?;
    let (reply, receive) = oneshot::channel();
    host.0
        .requests
        .try_send(Request::Read {
            device: device.id.clone(),
            query,
            reply,
        })
        .map_err(|_| Fault::unavailable())?;
    let value = tokio::time::timeout(Duration::from_secs(4), receive)
        .await
        .map_err(|_| Fault::unavailable())?
        .map_err(|_| Fault::unavailable())??;
    if !host.authorized(&device.id) {
        return Err(unauthorized());
    }
    let etag = value
        .get("version")
        .and_then(Value::as_str)
        .and_then(|version| HeaderValue::from_str(version).ok());
    let mut response = Json(value).into_response();
    if let Some(etag) = etag {
        response.headers_mut().insert(header::ETAG, etag);
    }
    Ok(response)
}

#[derive(Default, Deserialize)]
struct Page {
    cursor: Option<usize>,
    archived: Option<bool>,
}

async fn system(State(host): State<Host>, headers: HeaderMap) -> Result<Response, Fault> {
    read(host, headers, Read::System).await
}
async fn workspaces(
    State(host): State<Host>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Response, Fault> {
    read(
        host,
        headers,
        Read::Workspaces {
            offset: page.cursor.unwrap_or(0),
        },
    )
    .await
}
async fn sessions(
    State(host): State<Host>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
    Query(page): Query<Page>,
) -> Result<Response, Fault> {
    read(
        host,
        headers,
        Read::Sessions {
            workspace,
            archived: page.archived.unwrap_or(false),
            offset: page.cursor.unwrap_or(0),
        },
    )
    .await
}
async fn session(
    State(host): State<Host>,
    headers: HeaderMap,
    Path(session): Path<u64>,
) -> Result<Response, Fault> {
    read(host, headers, Read::Session(session)).await
}
async fn metadata(
    State(host): State<Host>,
    headers: HeaderMap,
    Path(session): Path<u64>,
) -> Result<Response, Fault> {
    read(host, headers, Read::Metadata(session)).await
}
async fn messages(
    State(host): State<Host>,
    headers: HeaderMap,
    Path(session): Path<u64>,
    Query(page): Query<Page>,
) -> Result<Response, Fault> {
    read(
        host,
        headers,
        Read::Messages {
            session,
            before: page.cursor,
        },
    )
    .await
}
async fn approvals(
    State(host): State<Host>,
    headers: HeaderMap,
    Path(session): Path<u64>,
) -> Result<Response, Fault> {
    read(host, headers, Read::Approvals(session)).await
}

fn submit(
    host: Host,
    headers: HeaderMap,
    route: &str,
    body: &[u8],
    command: Write,
) -> Result<(StatusCode, Json<Value>), Fault> {
    let device = auth(&host, &headers)?;
    check_csrf(&device, &headers)?;
    if header_text(&headers, "x-conduit-epoch") != Some(host.epoch()) {
        return Err(Fault::conflict(
            "The desktop restarted. Refresh before sending a new request.",
        ));
    }
    let key = header_text(&headers, "idempotency-key")
        .filter(|key| {
            (16..=128).contains(&key.len())
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        })
        .ok_or_else(invalid)?;
    let expires = header_text(&headers, "x-conduit-request-expires")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(invalid)?;
    let current = now();
    let mut digest = Hmac::<Sha256>::new_from_slice(host.0.fingerprint_key.as_bytes())
        .expect("HMAC accepts arbitrary key sizes");
    digest.update(route.as_bytes());
    digest.update(&[0]);
    digest.update(header_text(&headers, "if-match").unwrap_or("").as_bytes());
    digest.update(&[0]);
    digest.update(&expires.to_be_bytes());
    digest.update(body);
    let fingerprint = digest.finalize().into_bytes().into();
    let mut identity = host.identity();
    if !host.active()
        || identity.device.as_ref().is_none_or(|device_session| {
            device_session.id != device.id || device_session.expires <= current
        })
    {
        return Err(unauthorized());
    }
    identity.operations.retain(|_, op| {
        op.expires.saturating_add(REQUEST_LIFETIME) >= current
            || !matches!(op.state, "succeeded" | "failed")
    });
    let scoped = operation_key(&device, key);
    if let Some(existing) = identity.operations.get(&scoped) {
        if existing.fingerprint != fingerprint {
            return Err(Fault::conflict(
                "This request identifier has already been used for a different operation.",
            ));
        }
    } else {
        if expires <= current || expires > current + REQUEST_LIFETIME {
            return Err(Fault::new(
                410,
                "request_expired",
                "The request expired. Check the conversation before sending again.",
            ));
        }
        if identity.operations.len() >= MAX_OPERATIONS {
            return Err(limit());
        }
        identity.operations.insert(
            scoped.clone(),
            Operation {
                fingerprint,
                expires,
                created: current,
                updated: current,
                state: "queued",
                result: None,
                error: None,
            },
        );
        if host
            .0
            .requests
            .try_send(Request::Write {
                device: device.id,
                operation: scoped.clone(),
                command,
            })
            .is_err()
        {
            identity.operations.remove(&scoped);
            return Err(Fault::unavailable());
        }
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"operation_id": key, "host_epoch": host.epoch()})),
    ))
}

use axum::body::Bytes;

fn decode<T: serde::de::DeserializeOwned>(headers: &HeaderMap, body: &[u8]) -> Result<T, Fault> {
    if header_text(headers, "content-type").and_then(|value| value.split(';').next())
        != Some("application/json")
    {
        return Err(invalid());
    }
    serde_json::from_slice(body).map_err(|_| invalid())
}

fn version(headers: &HeaderMap) -> Result<String, Fault> {
    header_text(headers, "if-match")
        .filter(|value| (3..=128).contains(&value.len()))
        .map(str::to_owned)
        .ok_or_else(|| {
            Fault::new(
                428,
                "version_required",
                "Refresh the conversation before changing it.",
            )
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    workspace_id: String,
    engine: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Send {
    text: String,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ApprovalBody {
    Decision {
        allow: bool,
    },
    Answer {
        answers: std::collections::BTreeMap<String, Vec<String>>,
    },
}

async fn create(
    State(host): State<Host>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, Fault> {
    let data: Create = decode(&headers, &body)?;
    submit(
        host,
        headers,
        "create",
        &body,
        Write::Create {
            workspace: data.workspace_id,
            engine: data.engine,
        },
    )
}
async fn send(
    State(host): State<Host>,
    headers: HeaderMap,
    Path(session): Path<u64>,
    body: Bytes,
) -> Result<impl IntoResponse, Fault> {
    let data: Send = decode(&headers, &body)?;
    if data.text.trim().is_empty() || data.text.len() > MAX_TEXT {
        return Err(invalid());
    }
    let version = version(&headers)?;
    submit(
        host,
        headers,
        &format!("send/{session}"),
        &body,
        Write::Send {
            session,
            version,
            text: data.text,
        },
    )
}
async fn cancel(
    State(host): State<Host>,
    headers: HeaderMap,
    Path((session, run)): Path<(u64, String)>,
) -> Result<impl IntoResponse, Fault> {
    submit(
        host,
        headers,
        &format!("cancel/{session}/{run}"),
        &[],
        Write::Cancel { session, run },
    )
}
async fn approval(
    State(host): State<Host>,
    headers: HeaderMap,
    Path((session, run, ticket)): Path<(u64, String, String)>,
    body: Bytes,
) -> Result<impl IntoResponse, Fault> {
    let action = match decode::<ApprovalBody>(&headers, &body)? {
        ApprovalBody::Decision { allow } => ApprovalAction::Decide { allow },
        ApprovalBody::Answer { answers } => ApprovalAction::Answer { answers },
    };
    submit(
        host,
        headers,
        &format!("approval/{session}/{run}/{ticket}"),
        &body,
        Write::Approval {
            session,
            run,
            ticket,
            action,
        },
    )
}
async fn patch_metadata(
    State(host): State<Host>,
    headers: HeaderMap,
    Path(session): Path<u64>,
    body: Bytes,
) -> Result<impl IntoResponse, Fault> {
    let patch: MetadataPatch = decode(&headers, &body)?;
    let version = version(&headers)?;
    submit(
        host,
        headers,
        &format!("metadata/{session}"),
        &body,
        Write::Metadata {
            session,
            version,
            patch,
        },
    )
}

fn operation_key(device: &Device, key: &str) -> String {
    format!("{}:{key}", device.id)
}

async fn operation(
    State(host): State<Host>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Fault> {
    let device = auth(&host, &headers)?;
    let identity = host.identity();
    let op = identity
        .operations
        .get(&operation_key(&device, &id))
        .ok_or_else(Fault::missing)?;
    Ok(Json(
        json!({"id": id, "host_epoch": host.epoch(), "state": op.state, "created_at": op.created,
        "updated_at": op.updated, "result": op.result, "error": op.error}),
    ))
}

async fn events(State(host): State<Host>, headers: HeaderMap) -> Result<impl IntoResponse, Fault> {
    let device = auth(&host, &headers)?;
    let permit = host
        .0
        .event_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| limit())?;
    let changes = host.0.revision.subscribe();
    let stream = futures_util::stream::unfold(
        (host, device, changes, permit, true),
        |(host, device, mut changes, permit, initial)| async move {
            if !initial {
                let _ = tokio::time::timeout(Duration::from_secs(12), changes.changed()).await;
            }
            if !host.authorized(&device.id) {
                return None;
            }
            let revision = *changes.borrow_and_update();
            let event = Event::default()
                .event("refresh")
                .id(revision.to_string())
                .data(host.epoch());
            Some((
                Ok::<_, std::convert::Infallible>(event),
                (host, device, changes, permit, false),
            ))
        },
    );
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}

#[cfg(test)]
mod tests;
