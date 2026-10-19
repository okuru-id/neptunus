//! Signed sessions (`signedSession` in a manifest, used by the SpotiFLAC download extensions). The gateway
//! hands out a session once the user passes a challenge in the browser; requests are then HMAC-signed
//! with it. The protocol follows SpotiFLAC-Mobile's host (rust_backend/crates/extensions/src/signed_session).
//!
//! State lives in `extensions-data/<id>/session.json`: `install_id`, `session_id`, `session_secret`,
//! `expires_at`, and while a challenge is open `auth_url`, `state`, `challenge_at` and the `grant`.
//! The challenge page ends on `spotiflac://session-grant?grant=..&state=..`; the in-app window
//! (`ext_session_verify`) catches it, drops the URL into `grant-inbox` and `ext_session_poll` completes the exchange.

use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

type Rec = Map<String, Value>;

const GATEWAY_TIMEOUT: Duration = Duration::from_secs(60);

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn iso(ms: u64) -> String {
    let (secs, milli) = (ms / 1000, ms % 1000);
    let (z, rem) = ((secs / 86400) as i64 + 719468, secs % 86400);
    let (era, doe) = (z.div_euclid(146097), z.rem_euclid(146097));
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// RFC 3339 -> unix seconds
fn parse_time(s: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d, h, mi, sec) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    let zone = s.get(19..)?.trim_start_matches(|c: char| c == '.' || c == ',' || c.is_ascii_digit());
    let offset = match zone.as_bytes().first()? {
        b'Z' | b'z' => 0,
        sign @ (b'+' | b'-') => {
            let (zh, zm) = zone[1..].split_once(':')?;
            (zh.parse::<i64>().ok()? * 3600 + zm.parse::<i64>().ok()? * 60) * if *sign == b'-' { -1 } else { 1 }
        }
        _ => return None,
    };
    let y = if m <= 2 { y - 1 } else { y };
    let (era, yoe) = (y.div_euclid(400), y.rem_euclid(400));
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + sec - offset)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

fn random_hex(n: usize) -> String {
    hex(&digest("random", vec![], vec![])[..n])
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn dec(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        match (b[i], b.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())) {
            (b'%', Some(v)) => (out.push(v), i += 3),
            (b'+', _) => (out.push(b' '), i += 1),
            (c, _) => (out.push(c), i += 1),
        };
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn param(text: &str, key: &str) -> Option<String> {
    text.split(['?', '&', '#', ' ']).find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| dec(v))).filter(|v| !v.trim().is_empty())
}

/// `(grant, state)` out of a callback URL (also when nested in `cb=`) or pasted text.
fn parse_callback(text: &str) -> Option<(String, String)> {
    let state = param(text, "state");
    match (param(text, "grant").or_else(|| param(text, "code")), state) {
        (Some(g), Some(s)) => Some((g.trim().to_owned(), s.trim().to_owned())),
        _ => param(text, "cb").and_then(|cb| parse_callback(&cb)),
    }
}

fn usable(rec: &Rec) -> bool {
    let s = |k: &str| rec.get(k).and_then(Value::as_str).unwrap_or("");
    !s("session_id").trim().is_empty()
        && !s("session_secret").trim().is_empty()
        && parse_time(s("expires_at")).is_none_or(|t| (now_ms() / 1000) as i64 <= t)
}

fn verify(url: &str) -> Value {
    json!({ "ok": false, "needsVerification": true, "auth_url": url, "open_auth_url": url, "error": "verification required" })
}

impl Host {
    fn cfg(&self, key: &str, default: &str) -> String {
        self.sess_cfg[key].as_str().filter(|s| !s.is_empty()).unwrap_or(default).to_owned()
    }

    fn endpoint(&self, name: &str, default: &str) -> String {
        self.sess_cfg["endpoints"][name].as_str().filter(|s| !s.is_empty()).unwrap_or(default).to_owned()
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("https://") {
            return path.to_owned();
        }
        format!("{}/{}", self.sess_cfg["baseUrl"].as_str().unwrap_or("").trim().trim_end_matches('/'), path.trim_start_matches('/'))
    }

    fn rec(&self) -> Rec {
        match read_json(&self.data.join("session.json")) {
            Value::Object(m) => m,
            _ => Rec::new(),
        }
    }

    fn save(&self, rec: &Rec) -> R<()> {
        std::fs::write(self.data.join("session.json"), Value::Object(rec.clone()).to_string()).map_err(|e| e.to_string())
    }

    fn adopt(&self, rec: &mut Rec, from: &Value) -> bool {
        let keys = ["session_id", "session_secret", "expires_at"];
        if keys.iter().any(|k| from[k].as_str().is_none_or(str::is_empty)) {
            return false;
        }
        for k in keys {
            rec.insert(k.into(), from[k].clone());
        }
        for k in ["auth_url", "state", "challenge_at", "grant"] {
            rec.remove(k);
        }
        true
    }

    fn json(&self, res: attohttpc::Response) -> R<Value> {
        serde_json::from_slice(&res.bytes().map_err(|e| e.to_string())?).map_err(|e| format!("invalid session response: {e}"))
    }

    /// Gets a session, or a challenge the user has to pass (`Some(url)`).
    fn bootstrap(&self) -> R<Option<String>> {
        let mut rec = self.rec();
        let now = now_ms() / 1000;
        let fresh = rec.get("challenge_at").and_then(Value::as_u64).is_some_and(|at| now.saturating_sub(at) < 180);
        if let Some(url) = rec.get("auth_url").and_then(Value::as_str).filter(|u| fresh && !u.is_empty()) {
            return Ok(Some(url.to_owned()));
        }
        if rec.get("install_id").and_then(Value::as_str).is_none_or(str::is_empty) {
            rec.insert("install_id".into(), random_hex(16).into());
        }
        let url = format!(
            "{}?app_version={}&install_id={}",
            self.url(&self.endpoint("bootstrap", "/bootstrap")),
            enc(&self.cfg("appVersion", "ext-1.0")),
            enc(rec["install_id"].as_str().unwrap_or(""))
        );
        let res = self.send(&url, "GET", "", &Map::new())?;
        if !res.is_success() {
            return Err(format!("signed-session bootstrap returned HTTP {}", res.status().as_u16()));
        }
        let body = self.json(res)?;
        if self.adopt(&mut rec, &body) {
            self.save(&rec)?;
            return Ok(None);
        }
        let text = |k: &str| body[k].as_str().unwrap_or("").to_owned();
        let mut auth = Some(text("auth_url")).filter(|u| !u.is_empty()).unwrap_or_else(|| text("challenge_url"));
        let mut state = random_hex(16);
        match param(&auth, "state") {
            Some(s) => state = s,
            None if !auth.is_empty() => auth = format!("{auth}{}state={}", if auth.contains('?') { '&' } else { '?' }, enc(&state)),
            None => {}
        }
        if auth.is_empty() && !text("challenge_id").is_empty() {
            let cb = self.cfg("callbackUrl", "spotiflac://session-grant");
            let cb = format!("{cb}{}cb_version=v2grant&state={}", if cb.contains('?') { '&' } else { '?' }, enc(&state));
            auth = format!("{}?id={}&cb={}", self.url(&self.endpoint("challenge", "/challenge")), enc(&text("challenge_id")), enc(&cb));
        }
        if auth.is_empty() {
            return Err("signed-session bootstrap did not return a session or verification challenge".into());
        }
        rec.insert("auth_url".into(), auth.clone().into());
        rec.insert("state".into(), state.into());
        rec.insert("challenge_at".into(), now.into());
        self.save(&rec)?;
        Ok(Some(auth))
    }

    /// Trades the grant the browser handed back for a session.
    fn exchange(&self, grant: &str) -> R<()> {
        let mut rec = self.rec();
        let grant = if grant.is_empty() { rec.get("grant").and_then(Value::as_str).unwrap_or("").to_owned() } else { grant.to_owned() };
        if grant.is_empty() {
            return Err("no pending grant".into());
        }
        let body = json!({ "grant": grant, "install_id": rec.get("install_id"), "app_version": self.cfg("appVersion", "ext-1.0"), "platform": self.cfg("platform", "extension") }).to_string();
        let headers = Map::from_iter([("content-type".to_owned(), json!("application/json"))]);
        for attempt in 0..3 {
            let res = self.send(&self.url(&self.endpoint("exchange", "/session/exchange")), "POST", &body, &headers)?;
            let status = res.status().as_u16();
            if status == 429 && attempt < 2 {
                std::thread::sleep(Duration::from_secs(retry_after(&res).clamp(1, 10)));
                continue;
            }
            if !res.is_success() {
                return Err(format!("session exchange failed: HTTP {status}"));
            }
            let body = self.json(res)?;
            if !self.adopt(&mut rec, &body) {
                return Err("session exchange response missing session fields".into());
            }
            return self.save(&rec);
        }
        Err("session exchange did not complete".into())
    }

    fn sign(&self, rec: &Rec, method: &str, url: &str, body: &str) -> Map<String, Value> {
        let s = |k: &str| rec.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let (id, secret) = (s("session_id"), s("session_secret"));
        let (ms, nonce) = (now_ms(), random_hex(12));
        let b64 = |bytes: Vec<u8>| base64::Engine::encode(&URL_SAFE_NO_PAD, bytes);
        let window = (ms / 1000) / self.sess_cfg["timeWindowSeconds"].as_u64().filter(|w| *w > 0).unwrap_or(300);
        let rolling = b64(digest("hmac256", format!("{window}:{id}").into_bytes(), secret.into_bytes()));
        let hash = hex(&digest("sha256", body.as_bytes().to_vec(), vec![]));
        let path = url.split_once("://").and_then(|(_, r)| r.find('/').map(|i| &r[i..])).unwrap_or("/").split(['?', '#']).next().unwrap_or("/");
        let (version, platform) = (self.cfg("appVersion", "ext-1.0"), self.cfg("platform", "extension"));
        let ts = iso(ms);
        let signing = [self.cfg("schemeLabel", "SPOTIFLAC-HMAC-V1"), method.into(), path.into(), String::new(), hash.clone(), ts.clone(), nonce.clone(), id.clone(), version.clone(), platform.clone()].join("\n");
        let signature = b64(digest("hmac256", signing.into_bytes(), rolling.into_bytes()));
        let prefix = self.cfg("headerPrefix", "X-Sig-").to_lowercase();
        [("session", id), ("timestamp", ts), ("nonce", nonce), ("body-sha256", hash), ("signature", signature), ("app-version", version), ("platform", platform)]
            .into_iter()
            .map(|(k, v)| (format!("{prefix}{k}"), Value::from(v)))
            .collect()
    }

    fn signed_send(&self, rec: &Rec, method: &str, path: &str, body: &str, extras: &Map<String, Value>) -> R<(String, attohttpc::Response)> {
        let url = self.url(path);
        let mut headers = self.sign(rec, method, &url, body);
        if !body.is_empty() {
            headers.insert("content-type".into(), "application/json".into());
        }
        headers.extend(extras.iter().map(|(k, v)| (k.to_lowercase(), v.clone())));
        // the gateway resolves the stream on its side, which can take longer than a plain page fetch. Only
        // downloads (a deadline beyond a plain call's) get that: a metadata lookup that hung for 60s would eat
        // the whole call (e.g. `checkAvailability`), leaving the extension's own fallbacks no time.
        let downloading = self.deadline.borrow().saturating_duration_since(Instant::now()) > AVAILABILITY_TIMEOUT;
        let timeout = if downloading { GATEWAY_TIMEOUT } else { self.timeout };
        let res = self.send_within(timeout, &url, method, body, &headers)?;
        Ok((url, res))
    }

    fn refresh(&self, rec: &Rec) {
        let path = self.endpoint("refresh", "");
        let due = parse_time(rec.get("expires_at").and_then(Value::as_str).unwrap_or("")).is_some_and(|t| t - ((now_ms() / 1000) as i64) <= 3600);
        if path.is_empty() || !due {
            return;
        }
        let body = json!({ "install_id": rec.get("install_id") }).to_string();
        let Ok((_, res)) = self.signed_send(rec, "POST", &path, &body, &Map::new()) else { return };
        if let (true, Ok(fresh)) = (res.is_success(), self.json(res)) {
            let mut latest = self.rec();
            for k in ["session_id", "session_secret", "expires_at"] {
                if fresh[k].as_str().is_some_and(|v| !v.is_empty()) {
                    latest.insert(k.into(), fresh[k].clone());
                }
            }
            let _ = self.save(&latest);
        }
    }

    fn signed_fetch(&self, method: &str, path: &str, body: &str, extras: &Value) -> R<Value> {
        let method = method.trim().to_uppercase();
        let extras = extras.as_object().cloned().unwrap_or_default();
        let mut rec = self.rec();
        if !usable(&rec) {
            if let Some(url) = self.bootstrap()? {
                return Ok(verify(&url));
            }
            rec = self.rec();
        }
        self.refresh(&rec);
        rec = self.rec();
        let (mut rebootstrapped, mut provider_retries) = (false, 0);
        loop {
            let (url, res) = self.signed_send(&rec, &method, path, body, &extras)?;
            let status = res.status().as_u16();
            let retry = retry_after(&res);
            let headers: Map<String, Value> = res.headers().iter().map(|(k, v)| (k.as_str().to_owned(), String::from_utf8_lossy(v.as_bytes()).into_owned().into())).collect();
            let text = String::from_utf8_lossy(&res.bytes().map_err(|e| e.to_string())?).into_owned();
            let err: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let field = |k: &str| err[k].as_str().unwrap_or("").trim().to_lowercase();
            let (origin, code, action) = (field("origin"), field("code"), field("action"));
            if status == 503 && origin == "provider" && code == "provider_unavailable" && err["retryable"] == true && field("retry_mode") == "same_operation" && provider_retries < 2 {
                provider_retries += 1;
                std::thread::sleep(Duration::from_secs(retry.clamp(1, 10)));
                continue;
            }
            let regate = origin == "gateway" && ((status == 401 && action == "bootstrap_session") || (status == 428 && action == "verify"));
            if regate {
                for k in ["session_id", "session_secret", "expires_at"] {
                    rec.remove(k);
                }
                self.save(&rec)?;
                if let Some(url) = self.bootstrap()? {
                    return Ok(verify(&url));
                }
                if !rebootstrapped {
                    (rebootstrapped, rec) = (true, self.rec());
                    continue;
                }
            }
            let mut out = json!({ "statusCode": status, "status": status, "ok": (200..300).contains(&status), "url": url, "body": text, "headers": headers, "retryAfterSeconds": retry.max(err["retry_after_seconds"].as_u64().unwrap_or(0)) });
            if !code.is_empty() || !origin.is_empty() || !action.is_empty() {
                out["error"] = err["error"].as_str().unwrap_or("").trim().into();
                out["code"] = code.to_uppercase().into();
                out["origin"] = origin.into();
                out["action"] = action.into();
                out["retryable"] = (err["retryable"] == true).into();
                out["retryMode"] = field("retry_mode").into();
            }
            return Ok(out);
        }
    }

    /// `session.status/clear/completeGrant/signedFetch` of the sandbox.
    pub(super) fn session(&self, method: &str, args: &str) -> String {
        let args: Vec<Value> = serde_json::from_str(args).unwrap_or_default();
        let s = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or("");
        let out = match method {
            "status" => Ok(json!({ "authenticated": usable(&self.rec()) })),
            "clear" => Ok(json!({ "success": std::fs::remove_file(self.data.join("session.json")).is_ok() })),
            "completeGrant" => self.exchange(s(0)).map(|_| json!({ "success": true })),
            "signedFetch" if args.len() >= 2 => self.signed_fetch(s(0), s(1), s(2), args.get(3).unwrap_or(&Value::Null)),
            _ => Err("unknown signed session method".into()),
        };
        match out {
            Ok(v) => v,
            Err(error) if method == "signedFetch" => json!({ "ok": false, "error": error }),
            Err(error) => json!({ "success": false, "error": error }),
        }
        .to_string()
    }
}

fn retry_after(res: &attohttpc::Response) -> u64 {
    res.headers().get("retry-after").and_then(|v| v.to_str().ok()?.trim().parse().ok()).unwrap_or(0)
}

// ---------------------------------------------------------------- the browser hand-off

fn inbox() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".local/share")))?;
    Some(base.join("id.neptunus.desktop").join("grant-inbox"))
}

fn save_grant(url: &str) {
    if let Some(path) = inbox() {
        let _ = std::fs::write(path, url);
    }
}

/// Opens the challenge of extension `id` (the one its last call asked for) in a window of this app; the
/// redirect to `spotiflac://session-grant?...` is caught before it leaves the webview. `false`: nothing pending.
#[tauri::command]
pub async fn ext_session_verify(app: AppHandle, id: String) -> R<bool> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    let rec = read_json(&app.state::<Exts>().data.join(&id).join("session.json"));
    let Some(url) = rec["auth_url"].as_str().and_then(|u| u.parse::<tauri::Url>().ok()).filter(|u| u.scheme() == "https") else { return Ok(false) };
    let _ = std::fs::remove_file(inbox().unwrap_or_default());
    if let Some(old) = app.get_webview_window("ext-verify") {
        let _ = old.close();
    }
    let handle = app.clone();
    WebviewWindowBuilder::new(&app, "ext-verify", WebviewUrl::External(url))
        .title("Verifikasi ekstensi")
        .inner_size(480.0, 680.0)
        .on_navigation(move |to| {
            if to.scheme() != "spotiflac" {
                return true;
            }
            save_grant(to.as_str());
            let handle = handle.clone();
            std::thread::spawn(move || handle.get_webview_window("ext-verify").map(|w| w.close()));
            false
        })
        .build()
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Finishes a verification once the browser handed a grant back; returns the extension it was for.
#[tauri::command]
pub async fn ext_session_poll(app: AppHandle, text: Option<String>) -> R<Option<String>> {
    crate::blocking(move || {
        let exts = app.state::<Exts>();
        let file = inbox().unwrap_or_default();
        let Some(text) = text.or_else(|| std::fs::read_to_string(&file).ok()) else { return Ok(None) };
        let _ = std::fs::remove_file(&file);
        let Some((grant, state)) = parse_callback(&text) else { return Ok(None) };
        for id in exts.ids() {
            let path = exts.data.join(&id).join("session.json");
            let mut rec = match read_json(&path) {
                Value::Object(m) if m.get("state").and_then(Value::as_str) == Some(state.as_str()) => m,
                _ => continue,
            };
            rec.insert("grant".into(), grant.into());
            std::fs::write(&path, Value::Object(rec).to_string()).map_err(|e| e.to_string())?;
            let res = exts.call(&id, "completeGrant", json!([]))?;
            return match res["success"] == true {
                true => Ok(Some(id)),
                false => Err(res["error"].as_str().unwrap_or("verification failed").to_owned()),
            };
        }
        Ok(None)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_roundtrip() {
        assert_eq!(iso(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        assert_eq!(parse_time("2023-11-14T22:13:20.123Z"), Some(1_700_000_000));
        assert_eq!(parse_time("2023-11-14T23:13:20+01:00"), Some(1_700_000_000));
    }

    #[test]
    fn callback() {
        let want = Some(("g r".to_owned(), "abc".to_owned()));
        assert_eq!(parse_callback("spotiflac://session-grant?cb_version=v2grant&grant=g%20r&state=abc"), want);
        assert_eq!(parse_callback("https://x/?cb=spotiflac%3A%2F%2Fsession-grant%3Fgrant%3Dg%2520r%26state%3Dabc"), want);
        assert_eq!(parse_callback("nothing"), None);
    }
}
