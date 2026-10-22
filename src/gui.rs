//! `ssgg gui`: serves the profile editor in `tools/profile-gui` and the small lighting API its live panel calls.
//!
//! Everything listens on 127.0.0.1 only. The GameSense server is deliberately not reused: it is a game-facing
//! endpoint with its own CORS rule, while this one needs no CORS at all because the page is served from the
//! same origin. A browser page on another origin cannot read the token, and a request that arrives under a
//! foreign `Host` (DNS rebinding) or `Origin` is refused before the token is looked at.
//!
//! `[EXPERIMENTAL]` The lighting calls are the ones `ssgg rgb` sends; only a solid colour on the wired Apex Pro
//! TKL 2023 (`0x1628`) is confirmed on hardware, so a keyboard that ignores a call still answers success.

use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::devices::keyboards::Keyboard;
use crate::devices::keyboards::apex_7_profile::{self, MacroRecordRegion};
use crate::devices::{DeviceManager, DeviceType};
use crate::rgb::Color;
use crate::{Error, Result};

const INDEX_HTML: &str = include_str!("../tools/profile-gui/index.html");
const APP_JS: &str = include_str!("../tools/profile-gui/app.js");
const LIVE_JS: &str = include_str!("../tools/profile-gui/live.js");
const MAIN_JS: &str = include_str!("../tools/profile-gui/main.js");
const PRODUCTS_JS: &str = include_str!("../tools/profile-gui/products.generated.js");
const STYLE_CSS: &str = include_str!("../tools/profile-gui/style.css");
const MACROS_HTML: &str = include_str!("../tools/profile-gui/macros.html");
const MACROS_JS: &str = include_str!("../tools/profile-gui/macros.js");

const TOKEN_HEADER: &str = "x-ssgg-token";
const JS: &str = "text/javascript; charset=utf-8";

/// The keyboard the live panel drives. One handle for the whole run: the HID collection is opened once.
pub struct KeyboardHandle {
    name: String,
    device: Mutex<Box<dyn Keyboard>>,
}

impl KeyboardHandle {
    pub fn name(&self) -> &str {
        &self.name
    }
}

struct GuiState {
    port: u16,
    token: String,
    /// `index.html` with the token in a `<meta>` tag: the page learns the token only by being served here.
    index: String,
    /// `macros.html` with the same token: the macro editor window talks to the session endpoints.
    macros_index: String,
    keyboard: Option<KeyboardHandle>,
    /// The profile the macro editor window is editing. One at a time, like the page's open file.
    session: Mutex<Option<MacroSession>>,
}

/// The macro editor session: the profile JSON a window is editing, its revision, and the payload the
/// codec made of it when it last posted, so a window can show the write side's own numbers.
struct MacroSession {
    profile: serde_json::Value,
    revision: u64,
    payload_bytes: usize,
    region: RegionShape,
    /// What the decode said about the bytes behind the records: GG's own per-macro structures share the
    /// block, and a write sends them back verbatim.
    note: Option<String>,
}

/// The macro record region as the session endpoints report it, with the names the page uses.
#[derive(Serialize, Clone)]
struct RegionShape {
    start: usize,
    len: usize,
    used_bytes: usize,
    free_bytes: usize,
    records: usize,
    /// The records a list needs are more bytes than the region has.
    over_capacity: bool,
    /// How many bytes too many [`Self::over_capacity`] counts.
    overflow_bytes: usize,
}

impl From<MacroRecordRegion> for RegionShape {
    fn from(region: MacroRecordRegion) -> Self {
        Self {
            start: region.start,
            len: region.len,
            used_bytes: region.used_bytes,
            free_bytes: region.free_bytes,
            records: region.records,
            over_capacity: region.over_capacity,
            overflow_bytes: region.used_bytes.saturating_sub(region.len),
        }
    }
}

#[derive(Deserialize)]
struct ColorBody {
    color: String,
}

#[derive(Deserialize)]
struct MacroSessionBody {
    profile: serde_json::Value,
}

#[derive(Deserialize)]
struct BrightnessBody {
    level: u8,
}

enum LightingOp {
    Color(Color),
    Brightness(u8),
}

impl LightingOp {
    async fn apply(self, keyboard: &mut dyn Keyboard) -> Result<()> {
        match self {
            Self::Color(color) => {
                keyboard.set_color(color).await?;
                keyboard.apply().await
            }
            Self::Brightness(level) => keyboard.set_brightness(level).await,
        }
    }
}

/// Open the first keyboard for the live lighting panel; `None` leaves the panel hidden.
pub fn open_first_keyboard(manager: &DeviceManager) -> Option<KeyboardHandle> {
    let info = manager.first_device_of_type(DeviceType::Keyboard)?;
    let mut device = match manager.open_keyboard(info) {
        Ok(device) => device,
        Err(e) => {
            warn!("live lighting unavailable, could not open {}: {e}", info.name);
            return None;
        }
    };
    if let Err(e) = device.initialize() {
        warn!("live lighting unavailable, could not initialise {}: {e}", info.name);
        return None;
    }
    Some(KeyboardHandle {
        name: info.name.to_string(),
        device: Mutex::new(device),
    })
}

/// Serve the editor and the lighting API on `listener` until the task is dropped or the process stops.
pub async fn serve(listener: tokio::net::TcpListener, keyboard: Option<KeyboardHandle>) -> Result<()> {
    let port = listener.local_addr()?.port();
    let token = new_token();
    let meta = format!("<meta name=\"ssgg-live-token\" content=\"{token}\">\n</head>");
    let state = Arc::new(GuiState {
        port,
        index: INDEX_HTML.replacen("</head>", &meta, 1),
        macros_index: MACROS_HTML.replacen("</head>", &meta, 1),
        token,
        keyboard,
        session: Mutex::new(None),
    });
    info!("profile editor on 127.0.0.1:{port}");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

fn router(state: Arc<GuiState>) -> Router {
    Router::new()
        .route("/api/status", get(status))
        .route("/api/rgb/color", post(color))
        .route("/api/rgb/brightness", post(brightness))
        .route("/api/rgb/off", post(off))
        .route("/api/macros/session", get(macro_session_get).post(macro_session_post))
        .fallback_service(get(asset).with_state(state.clone()))
        .with_state(state)
}

/// 128 bits from the per-process random keys of std's `RandomState`.
///
/// shortcut: not a CSPRNG; enough to stop a web page from guessing a loopback token, swap for `getrandom` if the
/// token ever guards more than lighting.
fn new_token() -> String {
    let mut token = String::with_capacity(32);
    for salt in 0u64..2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(salt);
        // Writing to a String cannot fail.
        let _ = write!(token, "{:016x}", hasher.finish());
    }
    token
}

/// The request names this server, under this port, and was not sent from another origin.
fn same_origin(headers: &HeaderMap, port: u16) -> bool {
    let allowed = |host: &str| {
        host.eq_ignore_ascii_case(&format!("127.0.0.1:{port}"))
            || host.eq_ignore_ascii_case(&format!("localhost:{port}"))
    };
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    if !allowed(host) {
        return false;
    }
    match headers.get(header::ORIGIN) {
        None => true,
        Some(origin) => origin
            .to_str()
            .ok()
            .and_then(|o| o.strip_prefix("http://"))
            .is_some_and(allowed),
    }
}

fn token_matches(headers: &HeaderMap, token: &str) -> bool {
    let Some(given) = headers.get(TOKEN_HEADER).map(|v| v.as_bytes()) else {
        return false;
    };
    let expected = token.as_bytes();
    given.len() == expected.len() && given.iter().zip(expected).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn authorized(state: &GuiState, headers: &HeaderMap) -> bool {
    same_origin(headers, state.port) && token_matches(headers, &state.token)
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

async fn asset(State(state): State<Arc<GuiState>>, headers: HeaderMap, uri: Uri) -> Response {
    if !same_origin(&headers, state.port) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let (content_type, body) = match uri.path() {
        "/" | "/index.html" => ("text/html; charset=utf-8", state.index.as_str()),
        "/app.js" => (JS, APP_JS),
        "/live.js" => (JS, LIVE_JS),
        "/main.js" => (JS, MAIN_JS),
        "/products.generated.js" => (JS, PRODUCTS_JS),
        "/style.css" => ("text/css; charset=utf-8", STYLE_CSS),
        "/macros.html" => ("text/html; charset=utf-8", state.macros_index.as_str()),
        "/macros.js" => (JS, MACROS_JS),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    // The index carries the token, so nothing here may be cached.
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-store"),
            // The page holds the lighting token: no frames, plugins or <base> rewrites inside it.
            (
                header::CONTENT_SECURITY_POLICY,
                "frame-src 'none'; object-src 'none'; base-uri 'none'",
            ),
        ],
        body.to_owned(),
    )
        .into_response()
}

async fn status(State(state): State<Arc<GuiState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    Json(serde_json::json!({ "keyboard": state.keyboard.as_ref().map(|k| k.name.as_str()) })).into_response()
}

async fn color(State(state): State<Arc<GuiState>>, headers: HeaderMap, Json(body): Json<ColorBody>) -> Response {
    if !authorized(&state, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    match Color::parse(body.color.trim().trim_start_matches('#')) {
        Some(color) => run_lighting(state, LightingOp::Color(color)).await,
        None => error_response(
            StatusCode::BAD_REQUEST,
            Error::Other(format!("Invalid color: {}", body.color)).to_string(),
        ),
    }
}

async fn brightness(
    State(state): State<Arc<GuiState>>,
    headers: HeaderMap,
    Json(body): Json<BrightnessBody>,
) -> Response {
    if !authorized(&state, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    run_lighting(state, LightingOp::Brightness(body.level.min(100))).await
}

async fn off(State(state): State<Arc<GuiState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    run_lighting(state, LightingOp::Color(Color::BLACK)).await
}

/// Encode a profile the macro editor posted, with the codec `ssgg onboard write` uses, and report what
/// its payload looks like. `[EXPERIMENTAL]`: the record grammar behind the numbers is reverse-engineered
/// (see `apex_7_profile::decode_macros`), so this is the codec's own answer, not a second opinion - a
/// profile the write would refuse is refused here, with the same words.
fn preview_profile(value: &serde_json::Value) -> Result<(Vec<u8>, MacroRecordRegion, Option<String>)> {
    let text = serde_json::to_string(value).map_err(|e| Error::Other(format!("the profile is not JSON: {e}")))?;
    let payload = apex_7_profile::encode_json(&text)?;
    let decoded = apex_7_profile::decode(&payload)?;
    Ok((payload, decoded.macro_region, decoded.macro_note))
}

async fn macro_session_get(State(state): State<Arc<GuiState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let session = state.session.lock();
    let Some(session) = session.as_ref() else {
        return error_response(
            StatusCode::NOT_FOUND,
            "no profile in the macro editor session - open the editor from the profile editor's Macro editor button",
        );
    };
    Json(serde_json::json!({
        "revision": session.revision,
        "profile": &session.profile,
        "bytes": session.payload_bytes,
        "region": &session.region,
        "note": &session.note,
    }))
    .into_response()
}

async fn macro_session_post(
    State(state): State<Arc<GuiState>>,
    headers: HeaderMap,
    Json(body): Json<MacroSessionBody>,
) -> Response {
    if !authorized(&state, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let (payload, region, note) = match preview_profile(&body.profile) {
        Ok(outcome) => outcome,
        Err(e) => {
            // A record list that needs more bytes than the block has is the capacity limit, and the codec
            // says so in numbers: answer with the status that means "too large".
            let reason = e.to_string();
            let status = if reason.contains("macro records need") {
                StatusCode::PAYLOAD_TOO_LARGE
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            return error_response(status, reason);
        }
    };
    let shape = RegionShape::from(region);
    let revision = {
        let mut session = state.session.lock();
        let revision = session.as_ref().map_or(1, |current| current.revision + 1);
        *session = Some(MacroSession {
            profile: body.profile.clone(),
            revision,
            payload_bytes: payload.len(),
            region: shape.clone(),
            note,
        });
        revision
    };
    Json(serde_json::json!({
        "ok": true,
        "revision": revision,
        "bytes": payload.len(),
        "region": shape,
        "note": &session_note(state.as_ref()),
    }))
    .into_response()
}

/// The note the session holds, read after the lock is released.
fn session_note(state: &GuiState) -> Option<String> {
    state.session.lock().as_ref().and_then(|s| s.note.clone())
}

/// Run one lighting call on the keyboard's own thread of the blocking pool: the HID writes block, and the
/// keyboard is behind a lock because a call takes `&mut`.
async fn run_lighting(state: Arc<GuiState>, op: LightingOp) -> Response {
    if state.keyboard.is_none() {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "no keyboard is connected");
    }
    let runtime = tokio::runtime::Handle::current();
    let outcome = tokio::task::spawn_blocking(move || -> Result<()> {
        let keyboard = state
            .keyboard
            .as_ref()
            .ok_or_else(|| Error::DeviceNotFound("no keyboard is connected".to_string()))?;
        let mut device = keyboard.device.lock();
        runtime.block_on(op.apply(device.as_mut()))
    })
    .await;
    match outcome {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(Err(e)) => error_response(StatusCode::BAD_GATEWAY, e.to_string()),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("lighting task failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    /// A schema b profile JSON with nothing in it: the payload the codec makes from it has an empty
    /// macro region, which is the state the macro editor starts from.
    fn empty_profile_json() -> serde_json::Value {
        use crate::devices::keyboards::apex_7_profile::PAYLOAD_LEN_B;
        let mut payload = vec![0u8; PAYLOAD_LEN_B];
        payload[..2].copy_from_slice(&apex_7_profile::SCHEMA_B.to_le_bytes());
        payload[apex_7_profile::NAME_OFFSET..apex_7_profile::NAME_OFFSET + 6].copy_from_slice(b"Macros");
        let crc_at = apex_7_profile::checksum_offset(PAYLOAD_LEN_B);
        let crc = apex_7_profile::compute_checksum(&payload[..crc_at]);
        payload[crc_at..].copy_from_slice(&crc.to_le_bytes());
        let profile = apex_7_profile::decode(&payload).expect("the synthetic payload decodes");
        let text = apex_7_profile::profile_json(&profile).expect("the report serializes");
        serde_json::from_str(&text).expect("the report is JSON")
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).expect("header value"));
        }
        map
    }

    #[test]
    fn same_origin_accepts_only_this_server_under_this_port() {
        assert!(same_origin(&headers(&[("host", "127.0.0.1:8765")]), 8765));
        assert!(same_origin(
            &headers(&[("host", "localhost:8765"), ("origin", "http://localhost:8765")]),
            8765
        ));
        assert!(!same_origin(&headers(&[]), 8765), "no Host");
        assert!(
            !same_origin(&headers(&[("host", "evil.example:8765")]), 8765),
            "DNS rebinding"
        );
        assert!(
            !same_origin(&headers(&[("host", "127.0.0.1:9999")]), 8765),
            "another port"
        );
        assert!(!same_origin(
            &headers(&[("host", "127.0.0.1:8765"), ("origin", "https://evil.example")]),
            8765
        ));
        assert!(
            !same_origin(&headers(&[("host", "127.0.0.1:8765"), ("origin", "null")]), 8765),
            "file:// page"
        );
    }

    #[test]
    fn token_must_match_exactly() {
        assert!(token_matches(&headers(&[(TOKEN_HEADER, "abc")]), "abc"));
        assert!(!token_matches(&headers(&[(TOKEN_HEADER, "abd")]), "abc"));
        assert!(!token_matches(&headers(&[(TOKEN_HEADER, "ab")]), "abc"));
        assert!(!token_matches(&headers(&[]), "abc"));
    }

    #[test]
    fn tokens_are_32_hex_digits_and_differ() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    /// The session endpoint answers with the codec's own numbers: the payload length it encodes to and
    /// The session endpoints answer with the codec's own numbers: the payload length it encodes to, the
    /// The session endpoints answer with the codec's own numbers: the payload length it encodes to, the
    /// region a record list fills, and the numbers a list over capacity is refused with.
    #[test]
    fn preview_encodes_a_profile_and_reports_its_macro_region() {
        let (payload, region, note) = preview_profile(&empty_profile_json()).expect("an empty profile encodes");
        let (start, block) = apex_7_profile::macro_region_bounds(apex_7_profile::SCHEMA_B);
        assert_eq!(payload.len(), apex_7_profile::PAYLOAD_LEN_B);
        assert_eq!(
            (region.start, region.len, region.records, region.used_bytes),
            (start, block, 0, 0),
            "the region is the whole macro block, {block} bytes from {start}, empty here"
        );
        assert_eq!((region.free_bytes, region.over_capacity, note), (block, false, None));

        // A payload whose block already holds one record: header, a push of `5`, its release - 8 + 2 x 4
        // bytes. Macro id 3 is carried by binding number 35 + 3 - 1 = 37.
        let mut data = vec![0u8; apex_7_profile::PAYLOAD_LEN_B];
        data[0..2].copy_from_slice(&apex_7_profile::SCHEMA_B.to_le_bytes());
        data[apex_7_profile::NAME_OFFSET..apex_7_profile::NAME_OFFSET + 6].copy_from_slice(b"Macros");
        let record: Vec<u8> = "77 04 03 00 01 01 88 13 02 22 01 01 02 22 01 00"
            .split_whitespace()
            .map(|word| u8::from_str_radix(word, 16).expect("hex byte"))
            .collect();
        data[start..start + record.len()].copy_from_slice(&record);
        let crc_at = apex_7_profile::checksum_offset(data.len());
        let crc = apex_7_profile::compute_checksum(&data[..crc_at]);
        data[crc_at..].copy_from_slice(&crc.to_le_bytes());
        let value = serde_json::from_str::<serde_json::Value>(
            &apex_7_profile::profile_json(&apex_7_profile::decode(&data).expect("the payload decodes"))
                .expect("the report serializes"),
        )
        .expect("the report is JSON");
        let (payload, region, note) = preview_profile(&value).expect("the record list encodes");
        assert_eq!(
            (region.records, region.used_bytes, region.free_bytes, note),
            (1, 16, block - 16, None),
            "the record fills 16 of the block's {block} bytes and the walk found nothing it could not read"
        );
        assert_eq!(payload, data, "the report writes back the bytes the read produced");
        let back = apex_7_profile::decode(&payload).expect("the payload re-decodes");
        assert_eq!(
            (back.macros.len(), back.macros[0].id, back.macros[0].index),
            (1, 3, 37),
            "the record reaches the bytes and carries the binding number of its macro id"
        );
        assert!(back.crc_ok, "the payload carries the checksum it hashes to");

        // Past the region's capacity, refused with the numbers the write would give: 300 records of 50
        // pushes are 300 x (8 + 50 x 4) = 62,400 bytes.
        let mut crowded = empty_profile_json();
        let press = |key: u8| format!("02 {key:02x} 01 01");
        crowded["macros"] = json!(
            (0..300)
                .map(|position| {
                    let key = 0x04 + (position % 0x70) as u8;
                    let id = (position as u16 % apex_7_profile::MACRO_ID_MAX) + 1;
                    let actions = (0..50)
                        .map(|_| json!({ "kind": "press", "key": key }))
                        .collect::<Vec<_>>();
                    // 8 header bytes + 50 x 4-byte pushes = 208, the same bytes `raw` carries.
                    let raw = format!("77 04 {:02x} 00 01 01 64 00 {}", id, vec![press(key); 50].join(" "));
                    json!({
                        "offset": start + position * 208,
                        "ordinal": position,
                        "index": 35 + usize::from(id) - 1,
                        "flags_ok": true,
                        "mode": 4,
                        "mode_name": "repeat",
                        "id": id,
                        "flags": 0x0101u64,
                        "interval_ms": 100,
                        "record_bytes": 208,
                        "action_count": 50,
                        "actions": actions,
                        "raw": raw,
                    })
                })
                .collect::<Vec<_>>()
        );
        let reason = preview_profile(&crowded)
            .expect_err("300 x 208 bytes do not fit in the block")
            .to_string();
        assert!(
            reason.contains("62400 bytes") && reason.contains(&format!("has {block}")),
            "the refusal names the bytes needed and the region: {reason}"
        );
    }

    fn request(port: u16, head: &str, body: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        let text = format!(
            "{head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(text.as_bytes()).expect("send");
        let mut reply = String::new();
        stream.read_to_string(&mut reply).expect("read");
        reply
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_api_needs_the_token_the_page_is_served_with_and_a_keyboard() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(serve(listener, None));
        let host = format!("Host: 127.0.0.1:{port}");

        let (
            page,
            foreign,
            no_token,
            ok_token,
            bad_color,
            macros_page,
            session_before,
            session_foreign,
            session_bad,
            session_post,
            session_after,
        ) = tokio::task::spawn_blocking(move || {
            let page = request(port, &format!("GET / HTTP/1.1\r\n{host}"), "");
            let token = page
                .split("name=\"ssgg-live-token\" content=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_default()
                .to_string();
            let json = "Content-Type: application/json";
            let token_header = TOKEN_HEADER;
            let auth = format!("{host}\r\n{json}\r\n{token_header}: {token}");
            let foreign = request(port, "GET /api/status HTTP/1.1\r\nHost: evil.example", "");
            let no_token = request(port, &format!("POST /api/rgb/off HTTP/1.1\r\n{host}\r\n{json}"), "");
            let ok_token = request(port, &format!("POST /api/rgb/off HTTP/1.1\r\n{auth}"), "");
            let bad_color = request(
                port,
                &format!("POST /api/rgb/color HTTP/1.1\r\n{auth}"),
                "{\"color\":\"not-a-colour\"}",
            );
            let macros_page = request(port, &format!("GET /macros.html HTTP/1.1\r\n{host}"), "");
            let session_before = request(port, &format!("GET /api/macros/session HTTP/1.1\r\n{auth}"), "");
            let session_foreign = request(
                port,
                "POST /api/macros/session HTTP/1.1\r\nHost: evil.example\r\nContent-Type: application/json",
                "{\"profile\":{}}",
            );
            let session_bad = request(
                port,
                &format!("POST /api/macros/session HTTP/1.1\r\n{auth}"),
                "{\"profile\":{}}",
            );
            let body = format!(
                "{{\"profile\":{}}}",
                serde_json::to_string(&empty_profile_json()).expect("profile json")
            );
            let session_post = request(port, &format!("POST /api/macros/session HTTP/1.1\r\n{auth}"), &body);
            let session_after = request(port, &format!("GET /api/macros/session HTTP/1.1\r\n{auth}"), "");
            (
                page,
                foreign,
                no_token,
                ok_token,
                bad_color,
                macros_page,
                session_before,
                session_foreign,
                session_bad,
                session_post,
                session_after,
            )
        })
        .await
        .expect("requests");

        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains("ssgg-live-token"));
        assert!(foreign.starts_with("HTTP/1.1 403"), "{foreign}");
        assert!(no_token.starts_with("HTTP/1.1 403"), "{no_token}");
        assert!(
            ok_token.starts_with("HTTP/1.1 503"),
            "authorised, but no keyboard: {ok_token}"
        );
        assert!(bad_color.starts_with("HTTP/1.1 400"), "{bad_color}");

        // The macro editor page is served with the same token and the same headers as the index, and its
        // session endpoints answer only with that token.
        assert!(macros_page.starts_with("HTTP/1.1 200"), "{macros_page}");
        assert!(macros_page.contains("macroEditorBox"), "the popup page markup");
        assert!(
            macros_page.contains("ssgg-live-token"),
            "the popup page carries the token"
        );
        assert!(
            session_before.starts_with("HTTP/1.1 404"),
            "no session before the editor posts one: {session_before}"
        );
        assert!(
            session_foreign.starts_with("HTTP/1.1 403"),
            "a foreign Host is refused: {session_foreign}"
        );
        assert!(
            session_bad.starts_with("HTTP/1.1 422"),
            "a body with no profile is refused: {session_bad}"
        );
        assert!(session_post.starts_with("HTTP/1.1 200"), "{session_post}");
        assert!(session_post.contains("\"ok\":true"), "{session_post}");
        assert!(session_post.contains("11696"), "the payload length the codec made");
        assert!(
            session_after.starts_with("HTTP/1.1 200") && session_after.contains("\"revision\":1"),
            "{session_after}"
        );
        assert!(
            session_after.contains("Macros"),
            "the session returns the profile the window edits: {session_after}"
        );
    }
}
