// Sandbox API for SpotiFLAC-style extensions (https://spotiflac.zarz.moe/docs#api-reference).
// Everything is synchronous; `__host` is the native side (src/ext.rs).
(function (host) {
  "use strict";
  const g = globalThis;
  const str = (v) => (v === undefined || v === null ? "" : String(v));
  const isObj = (v) => v !== null && typeof v === "object" && !Array.isArray(v);

  g.registerExtension = (value) => { g.extension = value; };
  // calls are synchronous, so timers just wait and run in place
  g.setTimeout = (fn, ms, ...a) => { if (ms > 0) host.sleep(Math.min(ms, 300000)); if (typeof fn === "function") fn(...a); return 0; };
  g.clearTimeout = () => {};

  // ---- logging
  const fmt = (args) => Array.from(args, (a) => (typeof a === "string" ? a : (() => { try { return JSON.stringify(a); } catch (_) { return String(a); } })())).join(" ");
  g.log = {};
  for (const l of ["debug", "info", "warn", "error"]) g.log[l] = function () { host.log(l, fmt(arguments)); };
  g.console = { log() { host.log("info", fmt(arguments)); }, info() { host.log("info", fmt(arguments)); },
    warn() { host.log("warn", fmt(arguments)); }, error() { host.log("error", fmt(arguments)); }, debug() { host.log("debug", fmt(arguments)); } };

  // ---- bytes / text
  function utf8(s) {
    const out = [];
    for (const ch of str(s)) {
      let c = ch.codePointAt(0);
      if (c < 0x80) out.push(c);
      else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
      else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
      else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    }
    return out;
  }
  const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  function b64enc(bytes) {
    let s = "";
    for (let i = 0; i < bytes.length; i += 3) {
      const n = (bytes[i] << 16) | ((bytes[i + 1] || 0) << 8) | (bytes[i + 2] || 0);
      s += B64[n >> 18] + B64[(n >> 12) & 63] + (i + 1 < bytes.length ? B64[(n >> 6) & 63] : "=") + (i + 2 < bytes.length ? B64[n & 63] : "=");
    }
    return s;
  }
  function b64dec(s) {
    s = str(s).replace(/[^A-Za-z0-9+/\-_]/g, "").replace(/-/g, "+").replace(/_/g, "/");
    const out = [];
    for (let i = 0; i < s.length; i += 4) {
      const n = (B64.indexOf(s[i]) << 18) | (B64.indexOf(s[i + 1]) << 12) | ((B64.indexOf(s[i + 2]) & 63) << 6) | (B64.indexOf(s[i + 3]) & 63);
      out.push(n >> 16);
      if (i + 2 < s.length) out.push((n >> 8) & 255);
      if (i + 3 < s.length) out.push(n & 255);
    }
    return out;
  }
  function decodeUtf8(bytes) {
    let s = "";
    for (let i = 0; i < bytes.length;) {
      const b = bytes[i++];
      let c = b;
      if (b >= 0xf0) c = ((b & 7) << 18) | ((bytes[i++] & 63) << 12) | ((bytes[i++] & 63) << 6) | (bytes[i++] & 63);
      else if (b >= 0xe0) c = ((b & 15) << 12) | ((bytes[i++] & 63) << 6) | (bytes[i++] & 63);
      else if (b >= 0xc0) c = ((b & 31) << 6) | (bytes[i++] & 63);
      s += String.fromCodePoint(c);
    }
    return s;
  }
  const latin1 = (s) => Array.from(str(s), (c) => c.charCodeAt(0) & 255);
  g.btoa = (s) => b64enc(latin1(s));
  g.atob = (s) => String.fromCharCode.apply(null, b64dec(s));
  g.TextEncoder = function () { this.encoding = "utf-8"; this.encode = (s) => new Uint8Array(utf8(s)); };
  g.TextDecoder = function (enc) { this.encoding = enc || "utf-8"; this.decode = (b) => (b ? decodeUtf8(Array.from(b)) : ""); };

  // ---- URL
  const enc = (s) => encodeURIComponent(s).replace(/%20/g, "+");
  const dec = (s) => { try { return decodeURIComponent(s.replace(/\+/g, " ")); } catch (_) { return s; } };
  function URLSearchParams(init) {
    const list = [];
    if (typeof init === "string") {
      for (const part of init.replace(/^\?/, "").split("&")) if (part) { const i = part.indexOf("="); list.push(i < 0 ? [dec(part), ""] : [dec(part.slice(0, i)), dec(part.slice(i + 1))]); }
    } else if (Array.isArray(init)) for (const [k, v] of init) list.push([str(k), str(v)]);
    else if (isObj(init)) for (const k of Object.keys(init)) list.push([k, str(init[k])]);
    this._l = list;
  }
  URLSearchParams.prototype = {
    append(k, v) { this._l.push([str(k), str(v)]); },
    set(k, v) { this.delete(k); this._l.push([str(k), str(v)]); },
    delete(k) { this._l = this._l.filter((e) => e[0] !== str(k)); },
    get(k) { const e = this._l.find((e) => e[0] === str(k)); return e ? e[1] : null; },
    getAll(k) { return this._l.filter((e) => e[0] === str(k)).map((e) => e[1]); },
    has(k) { return this._l.some((e) => e[0] === str(k)); },
    forEach(fn) { for (const [k, v] of this._l) fn(v, k, this); },
    entries() { return this._l.map((e) => [e[0], e[1]])[Symbol.iterator](); },
    keys() { return this._l.map((e) => e[0])[Symbol.iterator](); },
    toString() { return this._l.map(([k, v]) => enc(k) + "=" + enc(v)).join("&"); },
  };
  URLSearchParams.prototype[Symbol.iterator] = URLSearchParams.prototype.entries;
  g.URLSearchParams = URLSearchParams;
  function URL(input, base) {
    let s = str(input);
    if (base !== undefined && !/^[a-z][a-z0-9+.-]*:/i.test(s)) {
      const b = new URL(base);
      if (s.startsWith("//")) s = b.protocol + s;
      else if (s.startsWith("/")) s = b.origin + s;
      else if (s.startsWith("?")) s = b.origin + b.pathname + s;
      else if (s.startsWith("#")) s = b.origin + b.pathname + b.search + s;
      else s = b.origin + b.pathname.replace(/[^/]*$/, "") + s;
    }
    const m = /^([a-z][a-z0-9+.-]*:)(?:\/\/(?:([^:@/]*)(?::([^@/]*))?@)?([^:/?#]*)(?::(\d+))?)?([^?#]*)(\?[^#]*)?(#.*)?$/i.exec(s);
    if (!m) throw new TypeError("Invalid URL: " + s);
    this.protocol = m[1].toLowerCase(); this.username = m[2] || ""; this.password = m[3] || "";
    this.hostname = (m[4] || "").toLowerCase(); this.port = m[5] || "";
    // resolve ./ and ../
    const segs = [];
    for (const p of (m[6] || "/").split("/")) { if (p === "..") segs.pop(); else if (p !== ".") segs.push(p); }
    this.pathname = segs.join("/") || "/";
    if (!this.pathname.startsWith("/")) this.pathname = "/" + this.pathname;
    this.searchParams = new URLSearchParams(m[7] || "");
    this.hash = m[8] || "";
  }
  Object.defineProperties(URL.prototype, {
    host: { get() { return this.hostname + (this.port ? ":" + this.port : ""); } },
    origin: { get() { return this.protocol + "//" + this.host; } },
    search: { get() { const q = this.searchParams.toString(); return q ? "?" + q : ""; },
      set(v) { this.searchParams = new URLSearchParams(str(v)); } },
    href: { get() { return this.origin + this.pathname + this.search + this.hash; } },
  });
  URL.prototype.toString = URL.prototype.toJSON = function () { return this.href; };
  g.URL = URL;

  // ---- network
  function headerMap(h) {
    const out = {};
    if (isObj(h)) for (const k of Object.keys(h)) if (h[k] !== undefined && h[k] !== null) out[k] = str(h[k]);
    return out;
  }
  function bodyText(b) {
    if (b === undefined || b === null) return "";
    if (typeof b === "string") return b;
    if (b instanceof URLSearchParams) return b.toString();
    return JSON.stringify(b);
  }
  function send(url, method, body, headers) {
    const r = JSON.parse(host.request(str(url), method, body, JSON.stringify(headers)));
    if (r.error !== undefined) return { ok: false, status: 0, statusCode: 0, statusText: "Network Error", error: r.error, body: "", headers: {} };
    r.statusCode = r.status;
    r.ok = r.status >= 200 && r.status < 300;
    return r;
  }
  function call(args, method, opts) {
    let body = "", headers = {};
    if (opts) {
      const o = args[1] || {};
      if (o.method) method = str(o.method).toUpperCase();
      body = bodyText(o.body); headers = headerMap(o.headers);
    } else if (method === "GET" || method === "DELETE") headers = headerMap(args[1]);
    else { body = bodyText(args[1]); headers = headerMap(args[2]); }
    if (body && !Object.keys(headers).some((k) => k.toLowerCase() === "content-type")) headers["Content-Type"] = "application/json";
    return send(args[0], method, body, headers);
  }
  g.http = {
    get() { return call(arguments, "GET"); },
    post() { return call(arguments, "POST"); },
    put() { return call(arguments, "PUT"); },
    delete() { return call(arguments, "DELETE"); },
    patch() { return call(arguments, "PATCH"); },
    request() { return call(arguments, "GET", true); },
    clearCookies() { host.clearCookies(); return true; },
  };
  g.fetch = function (url, opts) {
    const r = call([url, opts], "GET", true);
    r.url = str(url);
    const text = r.body;
    r.text = () => text;
    r.json = () => { try { return JSON.parse(text); } catch (_) { return undefined; } };
    r.arrayBuffer = () => (r.bin !== undefined ? b64dec(r.bin) : utf8(text));
    return r;
  };

  // ---- storage (per extension, persisted as JSON)
  const store = (secret) => ({
    get(k, fallback) { const v = host.storage("get", secret, str(k), ""); return v === "" ? fallback : JSON.parse(v); },
    set(k, v) { host.storage("set", secret, str(k), JSON.stringify(v === undefined ? null : v)); return true; },
    store(k, v) { host.storage("set", secret, str(k), JSON.stringify(v === undefined ? null : v)); return { success: true }; },
    remove(k) { host.storage("remove", secret, str(k), ""); return true; },
    has(k) { return host.storage("get", secret, str(k), "") !== ""; },
  });
  g.storage = store(false);
  g.credentials = store(true);

  // ---- files (relative paths live in the extension's data dir; absolute ones only under the download dir)
  g.file = {
    download(url, path, opts) {
      opts = opts || {};
      const cb = typeof opts.onProgress === "function" ? opts.onProgress : null;
      return JSON.parse(host.download(str(url), str(path), JSON.stringify(headerMap(opts.headers)), cb));
    },
    exists(p) { return host.file("exists", str(p), "") === "true"; },
    read(p) { return host.file("read", str(p), ""); },
    write(p, data) { return host.file("write", str(p), typeof data === "string" ? data : JSON.stringify(data)) === "true"; },
    delete(p) { return host.file("delete", str(p), "") === "true"; },
    getSize(p) { return Number(host.file("size", str(p), "")) || 0; },
    // downloadSegments@1: a segmented stream (each part a URL or {url, headers}) joined into one file
    downloadSegments(list, path, opts) {
      opts = opts || {};
      const cb = typeof opts.onProgress === "function" ? opts.onProgress : null;
      return JSON.parse(host.segments(JSON.stringify(list || []), str(path), JSON.stringify(headerMap(opts.headers)), cb));
    },
    // patternedFileTransform@1: decrypt every n-th fixed-size segment (Deezer's Blowfish-CBC streams)
    transformPatternedBlocks(src, dst, opts, onProgress) {
      return JSON.parse(host.transform(str(src), str(dst), JSON.stringify(opts || {}), typeof onProgress === "function" ? onProgress : undefined));
    },
  };

  // remux/convert local files with the ffmpeg on the PATH (desktop)
  g.ffmpeg = {
    convert: (src, dst, opts) => JSON.parse(host.ffmpeg(str(src), str(dst), JSON.stringify(opts || {}))),
  };

  // ---- signed sessions (manifest `signedSession`; see src/ext/session.rs)
  const sess = (method, args) => JSON.parse(host.session(method, JSON.stringify(args)));
  g.session = {
    status: () => sess("status", []),
    clear: () => sess("clear", []),
    completeGrant: (grant) => sess("completeGrant", grant === undefined ? [] : [str(grant)]),
    signedFetch(method, path, body, headers) {
      if (arguments.length < 2) return sess("signedFetch", []);
      try { return sess("signedFetch", [str(method), str(path), typeof body === "string" ? body : body == null ? "" : JSON.stringify(body), headerMap(headers)]); }
      catch (e) { return { ok: false, error: String(e) }; }
    },
  };

  // ---- utils
  const hex = (bytes) => bytes.map((b) => (b < 16 ? "0" : "") + b.toString(16)).join("");
  g.utils = {
    isDownloadCancelled: () => false,
    isRequestCancelled: () => false,
    randomUserAgent: () => host.userAgent(true),
    appUserAgent: () => host.userAgent(false),
    appVersion: () => host.appVersion(),
    setDownloadStatus() {},
    getResolutionRemainingMs: () => 60000,
    base64Encode: (s) => b64enc(utf8(s)),
    base64Decode: (s) => decodeUtf8(b64dec(s)),
    md5: (s) => hex(host.digest("md5", utf8(s), [])),
    sha256: (s) => hex(host.digest("sha256", utf8(s), [])),
    hmacSHA256: (m, k) => hex(host.digest("hmac256", utf8(m), utf8(k))),
    hmacSHA256Base64: (m, k) => b64enc(host.digest("hmac256", utf8(m), utf8(k))),
    hmacSHA1: (k, m) => host.digest("hmac1", typeof m === "string" ? utf8(m) : Array.from(m), typeof k === "string" ? utf8(k) : Array.from(k)),
    parseJSON(v) { try { return JSON.parse(str(v)); } catch (_) { return undefined; } },
    stringifyJSON(v) { try { return JSON.stringify(v); } catch (_) { return ""; } },
    sleep(ms) { if (ms > 0) host.sleep(Math.min(Math.trunc(ms), 300000)); return true; },
    generateKey(n) { const b = host.digest("random", [], []).slice(0, n || 32); return { success: true, key: b64enc(b), hex: hex(b) }; },
  };

  // ---- matching
  const norm = (s) => str(s).toLowerCase()
    .replace(/\s*[([].*?(remaster|explicit|feat|ft\.|version|edit|mix|live|deluxe).*?[)\]]/g, "")
    .replace(/[^\p{L}\p{N}]+/gu, " ").trim();
  function similarity(a, b) {
    a = norm(a); b = norm(b);
    if (a === b) return 1;
    if (!a || !b) return 0;
    const prev = Array.from({ length: b.length + 1 }, (_, i) => i);
    for (let i = 1; i <= a.length; i++) {
      let diag = prev[0]; prev[0] = i;
      for (let j = 1; j <= b.length; j++) {
        const t = prev[j];
        prev[j] = Math.min(prev[j] + 1, prev[j - 1] + 1, diag + (a[i - 1] === b[j - 1] ? 0 : 1));
        diag = t;
      }
    }
    return 1 - prev[b.length] / Math.max(a.length, b.length);
  }
  g.matching = {
    compareStrings: similarity,
    compareDuration: (a, b, tol) => Math.abs(+a - +b) <= (tol === undefined ? 3000 : +tol),
    normalizeString: norm,
  };

  g.gobackend = {
    sanitizeFilename: (s) => str(s).replace(/[\\/:*?"<>|\x00-\x1f]/g, "_").trim().slice(0, 200),
    getLocalTime() {
      const d = new Date();
      return { year: d.getFullYear(), month: d.getMonth() + 1, day: d.getDate(), hour: d.getHours(), minute: d.getMinutes(),
        second: d.getSeconds(), weekday: d.getDay(), offsetMinutes: d.getTimezoneOffset(), timezone: host.timezone(),
        timestamp: Math.floor(d.getTime() / 1000) };
    },
    buildFilename(t, m) { return str(t).replace(/\{(\w+)\}/g, (_, k) => str(m && m[k])); },
  };

  // Host entry point: runs one exported function; results travel as JSON.
  g.__call = function (method, argsJson) {
    const ext = g.extension;
    if (!ext || typeof ext[method] !== "function") throw new Error("Function " + method + " is not defined");
    const args = JSON.parse(argsJson);
    if (method === "download") args.splice(3, 0, () => {}); // progress comes from file.download
    const out = ext[method].apply(ext, args);
    return JSON.stringify(out === undefined ? null : out);
  };
})(__host);
