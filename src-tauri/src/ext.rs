//! Extensions in the SpotiFLAC format (https://spotiflac.zarz.moe/docs): a zip (`.spotiflac-ext`/`.sflx`)
//! with `manifest.json` + `index.js`, run in QuickJS. Each loaded extension owns one thread with its own
//! JS context; calls are queued to it. The sandbox API lives in `ext_prelude.js`.
//!
//! Disk layout under the app data dir: `extensions/<id>/` (unpacked package) and
//! `extensions-data/<id>/` (`settings.json`, `storage.json`, `disabled`, `files/`).

use base64::{Engine, engine::general_purpose::STANDARD};
use rquickjs::{Context, Ctx, Function, Runtime};
use serde_json::{Map, Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State};

pub mod session;
mod mp4;

const PRELUDE: &str = include_str!("ext_prelude.js");
const UA: &str = concat!("Neptunus/", env!("CARGO_PKG_VERSION"));
const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36";
/// One call (and its HTTP requests) may run this long; downloads get longer.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Finding a song on a download source (`checkAvailability`) can take many sequential lookups (Qobuz walks
/// candidate albums one by one), and it is part of a download the user started.
const AVAILABILITY_TIMEOUT: Duration = Duration::from_secs(120);

type R<T> = Result<T, String>;

struct Job {
    method: String,
    args: String,
    /// download progress, 0-100
    progress: Option<Box<dyn Fn(f64) + Send>>,
    /// absolute paths the extension may write to during this call
    root: Option<PathBuf>,
    reply: mpsc::Sender<R<String>>,
}

pub struct Exts {
    code: PathBuf,
    data: PathBuf,
    workers: Mutex<HashMap<String, mpsc::Sender<Job>>>,
}

/// For `ext-activity` events, sent from extension threads that have no `AppHandle` of their own.
static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

#[derive(Clone, serde::Serialize)]
struct Activity {
    ext: String,
    text: String,
}

/// Tells the UI what extension `ext` is doing right now (a step, a log line, a request), so a
/// download that sits at 0% still shows it is working.
pub(crate) fn activity(ext: &str, text: impl Into<String>) {
    if let Some(app) = APP.get() {
        let _ = app.emit("ext-activity", Activity { ext: ext.to_owned(), text: text.into() });
    }
}

pub fn init(app: &AppHandle) -> R<Exts> {
    let _ = APP.set(app.clone());
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(Exts { code: dir.join("extensions"), data: dir.join("extensions-data"), workers: Mutex::default() })
}

fn read_json(path: &Path) -> Value {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
}

impl Exts {
    fn manifest(&self, id: &str) -> R<Value> {
        match read_json(&self.code.join(id).join("manifest.json")) {
            Value::Object(m) => Ok(Value::Object(m)),
            _ => Err(format!("extension {id} is not installed")),
        }
    }

    fn enabled(&self, id: &str) -> bool {
        !self.data.join(id).join("disabled").exists()
    }

    fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = std::fs::read_dir(&self.code)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join("manifest.json").is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        ids.sort();
        ids
    }

    /// Saved settings over the manifest defaults: what `initialize()` receives.
    fn settings(&self, id: &str, manifest: &Value) -> Map<String, Value> {
        let mut out = Map::new();
        for s in manifest["settings"].as_array().into_iter().flatten() {
            if let (Some(k), d) = (s["key"].as_str(), &s["default"]) {
                if !d.is_null() {
                    out.insert(k.to_owned(), d.clone());
                }
            }
        }
        if let Value::Object(saved) = read_json(&self.data.join(id).join("settings.json")) {
            out.extend(saved);
        }
        out
    }

    /// Stops the extension's thread; the next call starts it again with fresh settings.
    fn unload(&self, id: &str) {
        self.workers.lock().unwrap().remove(id);
    }

    pub fn call(&self, id: &str, method: &str, args: Value) -> R<Value> {
        self.call_with(id, method, args, None, None)
    }

    fn call_with(&self, id: &str, method: &str, args: Value, progress: Option<Box<dyn Fn(f64) + Send>>, root: Option<PathBuf>) -> R<Value> {
        if !self.enabled(id) {
            return Err(format!("extension {id} is disabled"));
        }
        let (reply, answer) = mpsc::channel();
        let job = Job { method: method.to_owned(), args: args.to_string(), progress, root, reply };
        let mut workers = self.workers.lock().unwrap();
        let tx = match workers.get(id) {
            Some(tx) => tx.clone(),
            None => {
                let tx = self.spawn(id)?;
                workers.insert(id.to_owned(), tx.clone());
                tx
            }
        };
        drop(workers);
        if tx.send(job).is_err() {
            self.unload(id);
            return Err("extension stopped".into());
        }
        let out = answer.recv().map_err(|_| {
            self.unload(id); // the thread died (e.g. out of memory); start over next time
            "extension stopped".to_owned()
        })??;
        serde_json::from_str(&out).map_err(|e| e.to_string())
    }

    fn spawn(&self, id: &str) -> R<mpsc::Sender<Job>> {
        let manifest = self.manifest(id)?;
        let script = std::fs::read_to_string(self.code.join(id).join("index.js")).map_err(|e| e.to_string())?;
        let settings = Value::Object(self.settings(id, &manifest));
        let data = self.data.join(id);
        let _ = std::fs::create_dir_all(data.join("files"));
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready) = mpsc::channel();
        let name = id.to_owned();
        std::thread::Builder::new()
            .name(format!("ext-{id}"))
            .stack_size(16 << 20)
            .spawn(move || {
                let host = Host::new(name, &manifest, data);
                let vm = match Vm::start(host, &script, &settings) {
                    Ok(vm) => {
                        let _ = ready_tx.send(Ok(()));
                        vm
                    }
                    Err(e) => return drop(ready_tx.send(Err(e))),
                };
                for mut job in rx {
                    let out = vm.run(&mut job);
                    let _ = job.reply.send(out);
                }
                vm.cleanup();
            })
            .map_err(|e| e.to_string())?;
        ready.recv().map_err(|_| "extension failed to start".to_owned())??;
        Ok(tx)
    }

    fn install_zip(&self, bytes: &[u8]) -> R<Value> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not an extension package: {e}"))?;
        let mut read = |name: &str| -> R<Vec<u8>> {
            let mut f = zip.by_name(name).map_err(|_| format!("package has no {name}"))?;
            let mut out = Vec::new();
            f.read_to_end(&mut out).map_err(|e| e.to_string())?;
            Ok(out)
        };
        let manifest: Value = serde_json::from_slice(&read("manifest.json")?).map_err(|e| format!("invalid manifest: {e}"))?;
        let script = read("index.js")?;
        let id = manifest["name"].as_str().unwrap_or_default().to_owned();
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err("invalid manifest: name must be a simple id".into());
        }
        let icon = manifest["icon"].as_str().and_then(|n| read(n).ok().map(|b| (n.to_owned(), b)));
        self.unload(&id);
        let dir = self.code.join(&id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("index.js"), script).map_err(|e| e.to_string())?;
        if let Some((name, bytes)) = icon {
            if let Some(file) = Path::new(&name).file_name() {
                let _ = std::fs::write(dir.join(file), bytes);
            }
        }
        self.info(&id)
    }

    fn info(&self, id: &str) -> R<Value> {
        let mut m = self.manifest(id)?;
        let icon = m["icon"].as_str().and_then(|n| Path::new(n).file_name().map(|f| self.code.join(id).join(f)));
        let icon = icon.and_then(|p| std::fs::read(&p).ok().map(|b| {
            let mime = if b.starts_with(&[0x89, b'P']) { "image/png" } else { "image/jpeg" };
            format!("data:{mime};base64,{}", STANDARD.encode(b))
        }));
        m["id"] = id.into();
        m["enabled"] = self.enabled(id).into();
        m["values"] = Value::Object(self.settings(id, &m));
        m["iconData"] = icon.into();
        Ok(m)
    }
}

// ---------------------------------------------------------------- host side of the sandbox

struct Host {
    id: String,
    /// allowed host patterns (`example.com`, `*.example.com`)
    network: Vec<String>,
    allow_http: bool,
    data: PathBuf,
    storage: RefCell<Map<String, Value>>,
    secrets: RefCell<Map<String, Value>>,
    cookies: RefCell<HashMap<String, HashMap<String, String>>>,
    timeout: Duration,
    deadline: RefCell<Instant>,
    root: RefCell<Option<PathBuf>>,
    progress: RefCell<Option<Box<dyn Fn(f64) + Send>>>,
    /// the manifest's `signedSession` block
    sess_cfg: Value,
}

impl Host {
    fn new(id: String, manifest: &Value, data: PathBuf) -> Host {
        let p = &manifest["permissions"];
        let load = |f: &str| match read_json(&data.join(f)) {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        let secs = manifest["capabilities"]["networkTimeoutSeconds"].as_u64().unwrap_or(20).clamp(5, 120);
        Host {
            id,
            network: p["network"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_lowercase)).collect(),
            allow_http: p["allowHttp"].as_bool().unwrap_or(false),
            storage: RefCell::new(load("storage.json")),
            secrets: RefCell::new(load("credentials.json")),
            data,
            cookies: RefCell::default(),
            timeout: Duration::from_secs(secs),
            deadline: RefCell::new(Instant::now() + CALL_TIMEOUT),
            root: RefCell::default(),
            progress: RefCell::default(),
            sess_cfg: manifest["signedSession"].clone(),
        }
    }

    fn allowed(&self, url: &str) -> R<String> {
        let (scheme, rest) = url.split_once("://").ok_or("invalid URL")?;
        if scheme != "https" && !(scheme == "http" && self.allow_http) {
            return Err(format!("only https is allowed: {url}"));
        }
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = host.rsplit('@').next().unwrap_or("").split(':').next().unwrap_or("").to_lowercase();
        let ok = self.network.iter().any(|p| match p.strip_prefix("*.") {
            Some(base) => host == base || host.ends_with(&format!(".{base}")),
            None => host == *p,
        });
        if ok { Ok(host) } else { Err(format!("network access denied: {host} is not in the manifest permissions")) }
    }

    fn cookie_header(&self, host: &str) -> String {
        self.cookies.borrow().iter()
            .filter(|(domain, _)| host == domain.as_str() || host.ends_with(&format!(".{domain}")))
            .flat_map(|(_, jar)| jar.iter().map(|(k, v)| format!("{k}={v}")))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn keep_cookies(&self, host: &str, headers: &attohttpc::header::HeaderMap) {
        for c in headers.get_all("set-cookie") {
            let Ok(c) = c.to_str() else { continue };
            let mut parts = c.split(';');
            let Some((k, v)) = parts.next().and_then(|kv| kv.split_once('=')) else { continue };
            let domain = parts
                .filter_map(|a| a.trim().split_once('='))
                .find(|(a, _)| a.eq_ignore_ascii_case("domain"))
                .map(|(_, d)| d.trim_start_matches('.').to_lowercase())
                .unwrap_or_else(|| host.to_owned());
            self.cookies.borrow_mut().entry(domain).or_default().insert(k.trim().to_owned(), v.trim().to_owned());
        }
    }

    fn send(&self, url: &str, method: &str, body: &str, headers: &Map<String, Value>) -> R<attohttpc::Response> {
        self.send_within(self.timeout, url, method, body, headers)
    }

    fn send_within(&self, timeout: Duration, url: &str, method: &str, body: &str, headers: &Map<String, Value>) -> R<attohttpc::Response> {
        self.http(timeout, true, url, method, body, headers)
    }

    /// `whole` caps the entire request, body included; without it only a stalled read times out, so a large
    /// file that keeps arriving is never cut off mid-transfer.
    fn http(&self, timeout: Duration, whole: bool, url: &str, method: &str, body: &str, headers: &Map<String, Value>) -> R<attohttpc::Response> {
        let host = self.allowed(url)?;
        let left = self.deadline.borrow().saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("timeout: extension took too long to respond".into());
        }
        let verb = attohttpc::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
        let mut req = attohttpc::RequestBuilder::new(verb, url)
            .read_timeout(timeout)
            .header("User-Agent", UA)
            .follow_redirects(true);
        if whole {
            req = req.timeout(timeout.min(left));
        }
        let cookies = self.cookie_header(&host);
        if !cookies.is_empty() {
            req = req.header("Cookie", cookies);
        }
        for (k, v) in headers {
            let name = attohttpc::header::HeaderName::from_bytes(k.as_bytes()).map_err(|e| e.to_string())?;
            req = req.try_header(name, v.as_str().unwrap_or_default()).map_err(|e| e.to_string())?;
        }
        let started = Instant::now();
        let res = if body.is_empty() { req.send() } else { req.text(body).send() };
        // NEPTUNUS_HTTP_LOG=1: one line per extension request, to see where a slow call spends its time
        if std::env::var_os("NEPTUNUS_HTTP_LOG").is_some() {
            let status = res.as_ref().map_or_else(|e| e.to_string(), |r| r.status().as_u16().to_string());
            eprintln!("[ext {}] {method} {} -> {status} in {:.1}s", self.id, url.split('?').next().unwrap_or(url), started.elapsed().as_secs_f32());
        }
        activity(&self.id, format!("{method} {}", url.split_once("://").map_or(url, |(_, r)| r).split('?').next().unwrap_or(url)));
        let res = res.map_err(|e| e.to_string())?;
        self.keep_cookies(&host, res.headers());
        Ok(res)
    }

    fn request(&self, url: &str, method: &str, body: &str, headers: &str) -> String {
        let headers: Map<String, Value> = serde_json::from_str(headers).unwrap_or_default();
        let res = match self.send(url, method, body, &headers) {
            Ok(r) => r,
            Err(e) => return json!({ "error": e }).to_string(),
        };
        let status = res.status();
        let mut hs = Map::new();
        for (k, v) in res.headers() {
            let v = Value::from(String::from_utf8_lossy(v.as_bytes()).into_owned());
            match hs.get_mut(k.as_str()) {
                Some(Value::Array(a)) => a.push(v),
                Some(old) => *old = Value::Array(vec![old.take(), v]),
                None => drop(hs.insert(k.as_str().to_owned(), v)),
            }
        }
        // the docs show `headers["Content-Type"]`; offer both spellings
        for (k, v) in hs.clone() {
            let title: String = k.split('-').map(|w| {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
            }).collect::<Vec<_>>().join("-");
            hs.entry(title).or_insert(v);
        }
        let bytes = match res.bytes() {
            Ok(b) => b,
            Err(e) => return json!({ "error": e.to_string() }).to_string(),
        };
        let mut out = json!({
            "status": status.as_u16(),
            "statusText": status.canonical_reason().unwrap_or(""),
            "headers": hs,
        });
        match String::from_utf8(bytes) {
            Ok(text) => out["body"] = text.into(),
            Err(e) => {
                out["body"] = String::from_utf8_lossy(e.as_bytes()).into_owned().into();
                out["bin"] = STANDARD.encode(e.as_bytes()).into();
            }
        }
        out.to_string()
    }

    /// Relative paths resolve inside `files/`; absolute ones only inside the current download folder.
    fn path(&self, p: &str) -> R<PathBuf> {
        let path = Path::new(p);
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(format!("file access denied: path '{p}' is outside sandbox"));
        }
        if path.is_absolute() {
            return match &*self.root.borrow() {
                Some(root) if path.starts_with(root) => Ok(path.to_owned()),
                _ => Err("file access denied: absolute paths are not allowed".into()),
            };
        }
        Ok(self.data.join("files").join(path))
    }

    fn download<'js>(&self, url: &str, path: &str, headers: &str, cb: Option<Function<'js>>) -> String {
        let run = || -> R<String> {
            let dest = self.path(path)?;
            if let Some(d) = dest.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            let headers: Map<String, Value> = serde_json::from_str(headers).unwrap_or_default();
            *self.deadline.borrow_mut() = Instant::now() + DOWNLOAD_TIMEOUT;
            let part = dest.with_extension("part");
            let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            let (mut got, mut total, mut buf, mut last) = (0u64, 0u64, vec![0u8; 64 * 1024], Instant::now());
            // a dropped connection resumes with a Range request where the server allows it
            let mut tries = 0;
            'fetch: loop {
                let mut hs = headers.clone();
                if got > 0 {
                    hs.insert("Range".into(), format!("bytes={got}-").into());
                }
                let res = self.http(self.timeout, false, url, "GET", "", &hs)?;
                if !res.is_success() {
                    return Err(format!("HTTP error {}", res.status().as_u16()));
                }
                if got > 0 && res.status().as_u16() != 206 {
                    return Err("download interrupted and the server cannot resume it".into());
                }
                if got == 0 {
                    total = res.headers().get("content-length").and_then(|v| v.to_str().ok()?.parse::<u64>().ok()).unwrap_or(0);
                }
                let mut body = res.split().2;
                loop {
                    let n = match body.read(&mut buf) {
                        Ok(n) => n,
                        Err(e) if tries < 5 && (total == 0 || got < total) => {
                            tries += 1;
                            eprintln!("[ext {}] download stalled at {got} bytes ({e}), resuming", self.id);
                            continue 'fetch;
                        }
                        Err(e) => return Err(e.to_string()),
                    };
                    if n == 0 {
                        break 'fetch;
                    }
                    std::io::Write::write_all(&mut out, &buf[..n]).map_err(|e| e.to_string())?;
                    got += n as u64;
                    if last.elapsed() > Duration::from_millis(250) {
                        last = Instant::now();
                        if let Some(cb) = &cb {
                            let _: rquickjs::Result<()> = cb.call((got as f64, total as f64));
                        }
                        if total > 0 {
                            self.report(got as f64 * 100.0 / total as f64);
                        }
                    }
                }
            }
            if total > 0 && got < total {
                return Err(format!("download incomplete: {got} of {total} bytes"));
            }
            drop(out);
            std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
            Ok(dest.to_string_lossy().into_owned())
        };
        match run() {
            Ok(p) => json!({ "success": true, "path": p }).to_string(),
            Err(e) => json!({ "success": false, "error": e }).to_string(),
        }
    }

    fn report(&self, pct: f64) {
        if let Some(p) = &*self.progress.borrow() {
            p(pct.clamp(0.0, 100.0));
        }
    }

    fn file(&self, op: &str, p: &str, data: &str) -> R<String> {
        let path = self.path(p)?;
        Ok(match op {
            "exists" => path.exists().to_string(),
            "read" => std::fs::read_to_string(&path).unwrap_or_default(),
            "size" => path.metadata().map(|m| m.len()).unwrap_or(0).to_string(),
            "delete" => std::fs::remove_file(&path).is_ok().to_string(),
            "write" => {
                if let Some(d) = path.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                std::fs::write(&path, data).is_ok().to_string()
            }
            _ => return Err(format!("unknown file operation {op}")),
        })
    }

    /// `file.transformPatternedBlocks`: rewrites `src` into `dst` segment by segment, decrypting every
    /// `transformEvery`-th segment (from `transformOffset`) and copying the rest. Deezer streams come this
    /// way: Blowfish-CBC on every third 2048-byte block, each with a fresh IV.
    fn transform<'js>(&self, src: &str, dst: &str, opts: &str, cb: Option<Function<'js>>) -> String {
        use blowfish::cipher::{BlockDecryptMut, KeyIvInit, block_padding::NoPadding};
        let run = || -> R<String> {
            let o: Value = serde_json::from_str(opts).map_err(|e| e.to_string())?;
            let s = |k: &str| o[k].as_str().unwrap_or("").to_lowercase();
            if (s("operation"), s("algorithm"), s("mode")) != ("decrypt".into(), "blowfish".into(), "cbc".into())
                || !matches!(s("padding").as_str(), "" | "none")
            {
                return Err(format!("unsupported transform: {} {} {}", s("operation"), s("algorithm"), s("mode")));
            }
            let bytes = |k: &str| -> R<Vec<u8>> {
                let v = o[k].as_str().unwrap_or("");
                match s(&format!("{k}Encoding")).as_str() {
                    "hex" => (0..v.len()).step_by(2).map(|i| u8::from_str_radix(v.get(i..i + 2).unwrap_or("x"), 16).map_err(|e| e.to_string())).collect(),
                    "base64" => STANDARD.decode(v).map_err(|e| e.to_string()),
                    _ => Ok(v.as_bytes().to_vec()),
                }
            };
            let (key, iv) = (bytes("key")?, bytes("iv")?);
            let size = o["segmentSize"].as_u64().filter(|n| *n > 0 && n % 8 == 0).ok_or("segmentSize must be a multiple of 8")? as usize;
            let every = o["transformEvery"].as_u64().unwrap_or(1).max(1);
            let offset = o["transformOffset"].as_u64().unwrap_or(0);
            let partial = o["transformPartial"].as_bool().unwrap_or(false);
            let (src, dst) = (self.path(src)?, self.path(dst)?);
            let total = src.metadata().map_err(|e| e.to_string())?.len();
            let mut input = std::io::BufReader::new(std::fs::File::open(&src).map_err(|e| e.to_string())?);
            let mut output = std::io::BufWriter::new(std::fs::File::create(&dst).map_err(|e| e.to_string())?);
            let (mut buf, mut index, mut done, mut last) = (vec![0u8; size], 0u64, 0u64, Instant::now());
            loop {
                // fill a whole segment (read may return less)
                let mut n = 0;
                while n < size {
                    match input.read(&mut buf[n..]).map_err(|e| e.to_string())? {
                        0 => break,
                        k => n += k,
                    }
                }
                if n == 0 {
                    break;
                }
                if index >= offset && (index - offset) % every == 0 && (n == size || partial) {
                    let whole = n - n % 8;
                    cbc::Decryptor::<blowfish::Blowfish>::new_from_slices(&key, &iv)
                        .map_err(|e| e.to_string())?
                        .decrypt_padded_mut::<NoPadding>(&mut buf[..whole])
                        .map_err(|e| e.to_string())?;
                }
                std::io::Write::write_all(&mut output, &buf[..n]).map_err(|e| e.to_string())?;
                index += 1;
                done += n as u64;
                if last.elapsed() > Duration::from_millis(250) {
                    last = Instant::now();
                    if let Some(cb) = &cb {
                        let _: rquickjs::Result<()> = cb.call((done as f64, total as f64));
                    }
                }
            }
            std::io::Write::flush(&mut output).map_err(|e| e.to_string())?;
            Ok(dst.to_string_lossy().into_owned())
        };
        match run() {
            Ok(p) => json!({ "success": true, "path": p }).to_string(),
            Err(e) => json!({ "success": false, "error": e }).to_string(),
        }
    }

    fn storage(&self, op: &str, secret: bool, key: &str, value: &str) -> String {
        let (map, file) = if secret { (&self.secrets, "credentials.json") } else { (&self.storage, "storage.json") };
        let mut m = map.borrow_mut();
        match op {
            "get" => return m.get(key).map(Value::to_string).unwrap_or_default(),
            "set" => drop(m.insert(key.to_owned(), serde_json::from_str(value).unwrap_or(Value::Null))),
            _ => drop(m.remove(key)),
        }
        let _ = std::fs::write(self.data.join(file), Value::Object(m.clone()).to_string());
        String::new()
    }
}

fn digest(alg: &str, msg: Vec<u8>, key: Vec<u8>) -> Vec<u8> {
    use hmac::{KeyInit, Mac};
    use sha2::Digest;
    match alg {
        "md5" => md5::Md5::digest(&msg).to_vec(),
        "sha256" => sha2::Sha256::digest(&msg).to_vec(),
        "hmac256" => {
            let mut m = hmac::Hmac::<sha2::Sha256>::new_from_slice(&key).unwrap();
            m.update(&msg);
            m.finalize().into_bytes().to_vec()
        }
        "hmac1" => {
            let mut m = hmac::Hmac::<sha1::Sha1>::new_from_slice(&key).unwrap();
            m.update(&msg);
            m.finalize().into_bytes().to_vec()
        }
        // "random": 64 bytes seeded by the OS through RandomState (good enough for client keys)
        _ => (0..8u64).flat_map(|i| std::hash::BuildHasher::hash_one(&std::collections::hash_map::RandomState::new(), i).to_le_bytes()).collect(),
    }
}

fn timezone() -> String {
    std::env::var("TZ").ok().filter(|t| !t.is_empty())
        .or_else(|| std::fs::read_link("/etc/localtime").ok().and_then(|p| {
            let s = p.to_string_lossy().into_owned();
            s.split_once("zoneinfo/").map(|(_, z)| z.to_owned())
        }))
        .or_else(|| std::fs::read_to_string("/etc/timezone").ok().map(|s| s.trim().to_owned()))
        .unwrap_or_else(|| "Local".into())
}

// ---------------------------------------------------------------- the JS side

struct Vm {
    _rt: Runtime,
    ctx: Context,
    host: std::rc::Rc<Host>,
}

fn js_err(ctx: &Ctx<'_>, e: rquickjs::Error) -> String {
    if matches!(e, rquickjs::Error::Exception) {
        let v = ctx.catch();
        if let Some(ex) = v.as_exception() {
            return ex.message().unwrap_or_else(|| "exception".into());
        }
        return v.get::<rquickjs::Coerced<String>>().map(|s| s.0).unwrap_or_else(|_| "exception".into());
    }
    e.to_string()
}

fn set<'js>(h: &rquickjs::Object<'js>, name: &str, f: rquickjs::Result<Function<'js>>) -> R<()> {
    f.and_then(|f| h.set(name, f)).map_err(|e| e.to_string())
}

impl Vm {
    fn start(host: Host, script: &str, settings: &Value) -> R<Vm> {
        let rt = Runtime::new().map_err(|e| e.to_string())?;
        rt.set_memory_limit(256 << 20);
        rt.set_max_stack_size(8 << 20);
        let host = std::rc::Rc::new(host);
        // stops runaway scripts (`while (true) {}`) once the call is over time
        let watch = host.clone();
        rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > *watch.deadline.borrow())));
        let ctx = Context::full(&rt).map_err(|e| e.to_string())?;
        let vm = Vm { _rt: rt, ctx, host };
        vm.ctx.with(|ctx| -> R<()> {
            let h = rquickjs::Object::new(ctx.clone()).map_err(|e| e.to_string())?;
            let set = |name: &str, f| set(&h, name, f);
            let host = vm.host.clone();
            set("request", Function::new(ctx.clone(), move |url: String, method: String, body: String, headers: String| host.request(&url, &method, &body, &headers)))?;
            let host = vm.host.clone();
            set("clearCookies", Function::new(ctx.clone(), move || host.cookies.borrow_mut().clear()))?;
            let host = vm.host.clone();
            set("storage", Function::new(ctx.clone(), move |op: String, secret: bool, key: String, value: String| host.storage(&op, secret, &key, &value)))?;
            let host = vm.host.clone();
            set("download", Function::new(ctx.clone(), move |url: String, path: String, headers: String, cb: rquickjs::function::Opt<rquickjs::Value<'_>>| {
                host.download(&url, &path, &headers, cb.0.and_then(|v| v.into_function()))
            }))?;
            let host = vm.host.clone();
            set("file", Function::new(ctx.clone(), move |op: String, path: String, data: String| host.file(&op, &path, &data).unwrap_or_default()))?;
            let host = vm.host.clone();
            set("transform", Function::new(ctx.clone(), move |src: String, dst: String, opts: String, cb: rquickjs::function::Opt<rquickjs::Value<'_>>| {
                host.transform(&src, &dst, &opts, cb.0.and_then(|v| v.into_function()))
            }))?;
            let host = vm.host.clone();
            set("session", Function::new(ctx.clone(), move |method: String, args: String| host.session(&method, &args)))?;
            let host = vm.host.clone();
            set("log", Function::new(ctx.clone(), move |level: String, msg: String| {
                eprintln!("[ext {}] {level}: {msg}", host.id);
                if level != "debug" {
                    activity(&host.id, msg);
                }
            }))?;
            set("sleep", Function::new(ctx.clone(), |ms: f64| std::thread::sleep(Duration::from_millis(ms.max(0.0) as u64))))?;
            set("userAgent", Function::new(ctx.clone(), |browser: bool| if browser { BROWSER_UA } else { UA }.to_owned()))?;
            set("appVersion", Function::new(ctx.clone(), || env!("CARGO_PKG_VERSION").to_owned()))?;
            set("timezone", Function::new(ctx.clone(), timezone))?;
            set("digest", Function::new(ctx.clone(), |alg: String, msg: Vec<u8>, key: Vec<u8>| digest(&alg, msg, key)))?;
            ctx.globals().set("__host", h).map_err(|e| e.to_string())?;
            ctx.eval::<(), _>(PRELUDE).map_err(|e| js_err(&ctx, e))?;
            ctx.eval::<(), _>(script).map_err(|e| js_err(&ctx, e))?;
            let ext: Value = ctx.globals().get::<_, rquickjs::Value>("extension").map(|v| if v.is_object() { json!(true) } else { Value::Null }).unwrap_or(Value::Null);
            if ext.is_null() {
                return Err("extension did not call registerExtension()".into());
            }
            Ok(())
        })?;
        // initialize(settings) is optional; a throw means the settings are not usable yet
        let _ = vm.call("initialize", &json!([settings]).to_string());
        Ok(vm)
    }

    fn call(&self, method: &str, args: &str) -> R<String> {
        self.ctx.with(|ctx| {
            let f: Function = ctx.globals().get("__call").map_err(|e| e.to_string())?;
            f.call::<_, String>((method, args)).map_err(|e| js_err(&ctx, e))
        })
    }

    fn run(&self, job: &mut Job) -> R<String> {
        let limit = match job.method.as_str() {
            "download" => DOWNLOAD_TIMEOUT,
            "checkAvailability" => AVAILABILITY_TIMEOUT,
            _ => CALL_TIMEOUT,
        };
        *self.host.deadline.borrow_mut() = Instant::now() + limit;
        *self.host.root.borrow_mut() = job.root.take();
        *self.host.progress.borrow_mut() = job.progress.take();
        let out = self.call(&job.method, &job.args);
        *self.host.progress.borrow_mut() = None;
        *self.host.root.borrow_mut() = None;
        out
    }

    fn cleanup(&self) {
        let _ = self.call("cleanup", "[]");
    }
}

// ---------------------------------------------------------------- commands

#[tauri::command]
pub fn ext_list(exts: State<Exts>) -> Vec<Value> {
    exts.ids().iter().filter_map(|id| exts.info(id).ok()).collect()
}

#[tauri::command]
pub async fn ext_install_file(app: AppHandle, path: String) -> R<Value> {
    crate::blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        app.state::<Exts>().install_zip(&bytes)
    })
    .await
}

#[tauri::command]
pub async fn ext_install_url(app: AppHandle, url: String, sha256: Option<String>) -> R<Value> {
    crate::blocking(move || {
        if !url.starts_with("https://") {
            return Err("extension downloads must use https".into());
        }
        let bytes = attohttpc::get(&url).header("User-Agent", UA).timeout(Duration::from_secs(60))
            .send().and_then(|r| r.error_for_status()).and_then(|r| r.bytes()).map_err(|e| e.to_string())?;
        if let Some(want) = sha256.map(|s| s.trim().trim_start_matches("sha256:").to_lowercase()).filter(|s| s.len() == 64) {
            use sha2::Digest;
            let got: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
            if got != want {
                return Err("checksum mismatch: the downloaded package is not the one the repository lists".into());
            }
        }
        app.state::<Exts>().install_zip(&bytes)
    })
    .await
}

/// A repository is a `registry.json` URL (or a GitHub repo URL that has one on its main branch).
#[tauri::command]
pub async fn ext_repo(url: String) -> R<Value> {
    crate::blocking(move || {
        let url = url.trim().trim_end_matches('/').to_owned();
        let url = match url.strip_prefix("https://github.com/") {
            Some(repo) if !url.ends_with(".json") => format!("https://raw.githubusercontent.com/{repo}/main/registry.json"),
            _ => url,
        };
        if !url.starts_with("https://") {
            return Err("repository URL must use https".into());
        }
        let reg: Value = attohttpc::get(&url).header("User-Agent", UA).timeout(Duration::from_secs(30))
            .send().and_then(|r| r.error_for_status()).and_then(|r| r.json()).map_err(|e| e.to_string())?;
        if !reg["extensions"].is_array() {
            return Err("this URL is not an extension repository (no \"extensions\" list)".into());
        }
        Ok(reg)
    })
    .await
}

#[tauri::command]
pub fn ext_remove(exts: State<Exts>, id: String) -> R<()> {
    exts.unload(&id);
    let _ = std::fs::remove_dir_all(exts.data.join(&id));
    std::fs::remove_dir_all(exts.code.join(&id)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ext_set_enabled(exts: State<Exts>, id: String, enabled: bool) -> R<()> {
    exts.unload(&id);
    let flag = exts.data.join(&id).join("disabled");
    if enabled {
        let _ = std::fs::remove_file(flag);
        Ok(())
    } else {
        std::fs::create_dir_all(exts.data.join(&id)).map_err(|e| e.to_string())?;
        std::fs::write(flag, b"").map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn ext_save_settings(exts: State<Exts>, id: String, values: Map<String, Value>) -> R<Value> {
    exts.unload(&id);
    std::fs::create_dir_all(exts.data.join(&id)).map_err(|e| e.to_string())?;
    std::fs::write(exts.data.join(&id).join("settings.json"), Value::Object(values).to_string()).map_err(|e| e.to_string())?;
    exts.info(&id)
}

/// Calls an exported function (`customSearch`, `getHomeFeed`, `getAlbum`, a settings button action, ...).
#[tauri::command]
pub async fn ext_call(app: AppHandle, id: String, method: String, args: Vec<Value>) -> R<Value> {
    crate::blocking(move || app.state::<Exts>().call(&id, &method, Value::Array(args))).await
}

#[derive(Clone, serde::Serialize)]
struct Progress {
    key: String,
    pct: f64,
}

/// Downloads a track with download provider `id` into the first library folder, then adds it to the
/// library. A track from another extension (`track.provider_id`) is first looked up with the provider's
/// `checkAvailability`. `key` tags the progress events (`ext-download`).
#[tauri::command]
pub async fn ext_download(app: AppHandle, id: String, track: Value, quality: Option<String>, key: String) -> R<crate::db::Track> {
    crate::blocking(move || fetch(&app, &id, &track, quality, &key)).await
}

/// Downloads the library song at `path` again with extension `id`. With the extension it first came
/// from, its own track id is reused; with any other one the song is looked up by title and artist.
/// When the new file lands elsewhere, playlists follow it and the old file goes to the trash.
#[tauri::command]
pub async fn ext_redownload(app: AppHandle, path: String, id: String, quality: Option<String>) -> R<crate::db::Track> {
    crate::blocking(move || {
        let db = app.state::<crate::db::Db>();
        let track = match db.origin(&path).map_err(|e| e.to_string())? {
            Some((_, t)) => serde_json::from_str(&t).map_err(|e| e.to_string())?,
            None => {
                let t = db.track(&path).map_err(|e| e.to_string())?.ok_or("song is not in the library")?;
                json!({ "id": "", "name": t.title, "artists": t.artist, "album_name": t.album,
                        "duration_ms": t.duration_secs * 1000, "provider_id": "library" })
            }
        };
        let new = fetch(&app, &id, &track, quality, &path)?;
        if new.path != path {
            db.move_path(&path, &new.path).map_err(|e| e.to_string())?;
            if Path::new(&path).is_file() {
                let _ = crate::trash(&path);
            }
        }
        Ok(new)
    })
    .await
}

/// A download the extension left for the host to decrypt (`decryption_key`, Amazon). FLAC-in-MP4 is
/// decrypted natively into a plain .flac; other codecs (Opus, Dolby) need ffmpeg on the PATH.
fn finish_stream(path: &Path, res: &Value) -> R<PathBuf> {
    let key = ["decryption_key"].iter().find_map(|k| res[k].as_str()).or_else(|| res["decryption"]["key"].as_str()).filter(|k| !k.is_empty());
    if let Some(flac) = mp4::to_flac(path, key)? {
        if flac != path {
            let _ = std::fs::remove_file(path);
        }
        return Ok(flac);
    }
    let Some(key) = key else { return Ok(path.to_owned()) };
    let ext = res["output_extension"].as_str().filter(|e| !e.is_empty()).unwrap_or(".m4a");
    let out = path.with_extension(format!("dec{ext}"));
    let ok = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-decryption_key", key, "-i"]).arg(path).args(["-c", "copy"]).arg(&out)
        .status()
        .map_err(|_| "Kualitas ini butuh ffmpeg untuk dibuka. Pasang ffmpeg, atau pilih kualitas FLAC.".to_owned())?;
    if !ok.success() {
        return Err("ffmpeg gagal mendekripsi file unduhan".into());
    }
    let _ = std::fs::remove_file(path);
    let dst = path.with_extension(ext.trim_start_matches('.'));
    std::fs::rename(&out, &dst).map_err(|e| e.to_string())?;
    Ok(dst)
}

pub(crate) fn fetch(app: &AppHandle, id: &str, track: &Value, quality: Option<String>, key: &str) -> R<crate::db::Track> {
    {
        let (id, key) = (id.to_owned(), key.to_owned());
        let exts = app.state::<Exts>();
        let manifest = exts.manifest(&id)?;
        let db = app.state::<crate::db::Db>();
        let dir = match db.folders().ok().and_then(|f| f.into_iter().next()) {
            Some(d) => PathBuf::from(d),
            None => app.path().audio_dir().map_err(|e| e.to_string())?.join("Neptunus"),
        };
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let text = |k: &str| track[k].as_str().unwrap_or_default().to_owned();
        let clean = |s: &str| -> String {
            let s: String = s.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c }).collect();
            s.trim().trim_matches('.').to_owned()
        };
        // <library>/<main artist>/<artist - title>.flac
        let artists = text("artists");
        let main = clean(artists.split([',', '&', ';']).next().unwrap_or_default());
        let folder = dir.join(if main.is_empty() { "Unknown Artist" } else { &main });
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let quality = quality
            .or_else(|| manifest["qualityOptions"][0]["id"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "LOSSLESS".into());
        let out = folder.join(format!("{}.flac", clean(&format!("{artists} - {}", text("name")))));
        let emit = {
            let (app, key) = (app.clone(), key.clone());
            move |pct: f64| drop(app.emit("ext-download", Progress { key: key.clone(), pct }))
        };
        emit(0.0);
        let options = json!({ "spotify_id": track["id"], "track": track, "isrc": track["isrc"],
                              "deezer_id": track["deezer_id"], "duration_ms": track["duration_ms"] });
        let mut track_id = text("id");
        if track["provider_id"].as_str().is_some_and(|p| p != id) {
            activity(&id, "Mencari lagu di sumber unduhan…");
            let found = exts.call(&id, "checkAvailability", json!([text("isrc"), text("name"), text("artists"), options]))?;
            track_id = found["track_id"].as_str().or_else(|| found["trackId"].as_str()).filter(|_| found["available"] != false)
                .ok_or("this song was not found on the download source")?.to_owned();
        }
        activity(&id, "Mengunduh…");
        let res = exts.call_with(&id, "download", json!([track_id, quality, out.to_string_lossy(), options]), Some(Box::new(emit)), Some(dir.clone()))?;
        if res["success"] != true {
            let msg = ["error_message", "errorMessage", "error"].iter().find_map(|k| res[k].as_str()).unwrap_or("download failed");
            return Err(msg.to_owned());
        }
        let path = ["file_path", "filePath", "path"].iter().find_map(|k| res[k].as_str()).map(PathBuf::from).unwrap_or(out);
        // encrypted / MP4-wrapped streams become a plain playable file
        let path = finish_stream(&path, &res)?;
        activity(&id, "Menyimpan tag dan sampul…");
        tag_file(&path, &track);
        // a cover lookup that came up empty before the tags were written is cached as "none"
        if let Ok(dir) = app.path().app_cache_dir().map(|d| d.join("covers")) {
            std::fs::read_dir(dir).into_iter().flatten().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "none")).for_each(|e| drop(std::fs::remove_file(e.path())));
        }
        let mut t = crate::read_track(&path.to_string_lossy()).ok_or("downloaded file is not playable audio")?;
        // provider metadata beats a bare file name when the file has no tags
        if t.artist.is_empty() {
            t.artist = text("artists");
            t.title = text("name");
            t.album = text("album_name");
        }
        db.upsert(std::slice::from_ref(&t)).map_err(|e| e.to_string())?;
        // the source track (with its provider_id) is what a later re-download starts from
        let _ = db.set_origin(&t.path, &id, &track.to_string());
        Ok(t)
    }
}

/// Extensions leave files without cover art or tags: fill what is missing from the catalog's metadata.
pub(crate) fn tag_file(path: &Path, track: &Value) {
    use lofty::{config::WriteOptions, file::TaggedFileExt, picture::{Picture, PictureType}, probe::Probe, tag::{Accessor, Tag, TagExt}};
    let Ok(mut file) = Probe::open(path).and_then(|p| p.read()) else { return };
    if file.primary_tag().is_none() {
        file.insert_tag(Tag::new(file.primary_tag_type()));
    }
    let Some(tag) = file.primary_tag_mut() else { return };
    let text = |k: &str| track[k].as_str().filter(|s| !s.is_empty());
    if tag.title().is_none() {
        text("name").inspect(|v| tag.set_title((*v).to_owned()));
    }
    if tag.artist().is_none() {
        text("artists").inspect(|v| tag.set_artist((*v).to_owned()));
    }
    if tag.album().is_none() {
        text("album_name").inspect(|v| tag.set_album((*v).to_owned()));
    }
    if tag.pictures().is_empty() {
        let url = ["cover_url", "images", "image_url"].iter().find_map(|k| text(k)).filter(|u| u.starts_with("https://"));
        let bytes = url.and_then(|u| attohttpc::get(u).header("User-Agent", UA).timeout(Duration::from_secs(20)).send().ok()?.bytes().ok());
        if let Some(mut pic) = bytes.and_then(|b| Picture::from_reader(&mut std::io::Cursor::new(b)).ok()) {
            pic.set_pic_type(PictureType::CoverFront);
            tag.push_picture(pic);
        }
    }
    let _ = tag.save_to_path(path, WriteOptions::default());
}

/// Lyrics from enabled `lyrics_provider` extensions, as LRC text (or plain text).
pub fn lyrics(app: &AppHandle, title: &str, artist: &str, album: &str, duration: f64) -> Option<String> {
    let exts = app.state::<Exts>();
    for id in exts.ids() {
        let Ok(m) = exts.manifest(&id) else { continue };
        if !exts.enabled(&id) || !m["type"].as_array().is_some_and(|t| t.iter().any(|t| t == "lyrics_provider")) {
            continue;
        }
        let Ok(res) = exts.call(&id, "fetchLyrics", json!([title, artist, album, duration])) else { continue };
        if res.is_null() || res["instrumental"] == true {
            continue;
        }
        let lines: Vec<String> = res["lines"].as_array().into_iter().flatten().filter_map(|l| {
            let ms = l["startTimeMs"].as_f64().or_else(|| l["start_time_ms"].as_f64())?;
            let words = l["words"].as_str().unwrap_or_default();
            let cs = (ms / 10.0) as u64;
            Some(format!("[{:02}:{:02}.{:02}]{words}", cs / 6000, cs / 100 % 60, cs % 100))
        }).collect();
        if !lines.is_empty() {
            return Some(lines.join("\n"));
        }
        if let Some(txt) = res["plainLyrics"].as_str().filter(|s| !s.trim().is_empty()) {
            return Some(txt.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `EXT_DIR=<dir with *.zip packages> cargo test ext_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn ext_smoke() {
        let dir = PathBuf::from(std::env::var("EXT_DIR").unwrap());
        let tmp = std::env::temp_dir().join("neptunus-ext-test");
        let exts = Exts { code: tmp.join("code"), data: tmp.join("data"), workers: Mutex::default() };
        for e in std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "zip")) {
            let id = exts.install_zip(&std::fs::read(e.path()).unwrap()).map(|m| m["id"].clone());
            println!("install {:?}: {id:?}", e.file_name());
        }
        let only = std::env::var("EXT_ONLY").ok();
        for id in exts.ids() {
            if only.as_deref().is_some_and(|o| o != id) {
                continue;
            }
            let t = Instant::now();
            let r = exts.call(&id, "customSearch", json!(["bohemian rhapsody", {}]))
                .or_else(|_| exts.call(&id, "searchTracks", json!(["bohemian rhapsody", 5])));
            let s = r.map(|v| v.to_string()).unwrap_or_else(|e| format!("ERR {e}"));
            println!("{id} search {:?}: {}", t.elapsed(), s.chars().take(300).collect::<String>());
            let r = exts.call(&id, "getHomeFeed", json!([]));
            let s = r.map(|v| v.to_string()).unwrap_or_else(|e| format!("ERR {e}"));
            println!("{id} home: {}", s.chars().take(200).collect::<String>());
        }
        if let Ok(dl) = std::env::var("EXT_DOWNLOAD") {
            // EXT_DOWNLOAD=<id>:<track id>
            let (id, track) = dl.split_once(':').unwrap();
            let out = tmp.join("dl");
            std::fs::create_dir_all(&out).unwrap();
            let r = exts.call_with(id, "download", json!([track, "", out.join("t.flac").to_string_lossy(), {}]),
                Some(Box::new(|p| print!("{p:.0}% "))), Some(out.clone()));
            println!("\ndownload: {r:?}");
        }
        if let Ok(l) = std::env::var("EXT_LYRICS") {
            let r = exts.call(&l, "fetchLyrics", json!(["Bohemian Rhapsody", "Queen", "", 354]));
            println!("lyrics: {}", r.map(|v| v.to_string()).unwrap_or_else(|e| e).chars().take(300).collect::<String>());
        }
    }

    /// Real install, real network: `EXT_LIVE=qobuz-web cargo test ext_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn ext_live() {
        let id = std::env::var("EXT_LIVE").unwrap();
        let base = PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share/id.neptunus.desktop");
        let exts = Exts { code: base.join("extensions"), data: base.join("extensions-data"), workers: Mutex::default() };
        let q = std::env::var("EXT_QUERY").unwrap_or_else(|_| "amnesia al ghazali".into());
        let r = exts.call(&id, "customSearch", json!([q, {}])).or_else(|_| exts.call(&id, "searchTracks", json!([q, 5])));
        let r = r.unwrap_or_else(|e| panic!("search: {e}"));
        let t = r["tracks"].as_array().and_then(|a| a.first()).or_else(|| r.as_array().and_then(|a| a.first())).cloned().expect("no track");
        println!("track: {t}");
        let out = std::env::temp_dir().join("neptunus-live");
        std::fs::create_dir_all(&out).unwrap();
        let opts = json!({ "spotify_id": t["id"], "track": t, "isrc": t["isrc"] });
        let r = exts.call_with(&id, "download", json!([t["id"], "LOSSLESS", out.join("t.flac").to_string_lossy(), opts]), Some(Box::new(|p| print!("{p:.0}% "))), Some(out.clone()));
        println!("\ndownload: {r:?}");
    }
}


#[cfg(test)]
mod transform_tests {
    use super::*;
    use blowfish::cipher::{BlockEncryptMut, KeyIvInit, block_padding::NoPadding};

    #[test]
    fn decrypts_every_third_segment() {
        let dir = std::env::temp_dir().join(format!("neptunus-transform-{}", std::process::id()));
        let host = Host::new("t".into(), &json!({}), dir.clone());
        let (key, iv) = (*b"0123456789abcdef", *b"\x00\x01\x02\x03\x04\x05\x06\x07");
        // 7 full segments and a partial one; the partial one must stay as it is
        let plain: Vec<u8> = (0..(2048 * 7 + 1000)).map(|i| (i * 31 % 251) as u8).collect();
        let mut enc = plain.clone();
        for (i, seg) in enc.chunks_mut(2048).enumerate() {
            if i % 3 == 0 && seg.len() == 2048 {
                cbc::Encryptor::<blowfish::Blowfish>::new_from_slices(&key, &iv).unwrap().encrypt_padded_mut::<NoPadding>(seg, 2048).unwrap();
            }
        }
        assert_ne!(enc, plain);
        std::fs::create_dir_all(dir.join("files")).unwrap();
        std::fs::write(dir.join("files/in.bin"), &enc).unwrap();
        let opts = json!({ "operation": "decrypt", "algorithm": "blowfish", "mode": "cbc", "key": "30313233343536373839616263646566",
            "keyEncoding": "hex", "iv": "0001020304050607", "ivEncoding": "hex", "padding": "none", "segmentSize": 2048,
            "transformEvery": 3, "transformOffset": 0, "transformPartial": false });
        let res: Value = serde_json::from_str(&host.transform("in.bin", "out.bin", &opts.to_string(), None)).unwrap();
        assert_eq!(res["success"], true, "{res}");
        assert_eq!(std::fs::read(dir.join("files/out.bin")).unwrap(), plain);
        let _ = std::fs::remove_dir_all(dir);
    }
}
