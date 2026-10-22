// Bridge Stremio addon → Neptunus video tab (video_provider).
// The addon base URL comes from the manifestUrl setting, so any Stremio addon on
// an allowed host works without code changes. Everything is synchronous QuickJS.
registerExtension({
  initialize(settings) {
    this.base = String(settings.manifestUrl || "").replace(/\/+$/, "");
    if (!/^https:\/\//.test(this.base)) throw new Error("URL manifest harus https");
  },

  videoInfo() {
    const m = json(this.base + "/manifest.json");
    const res = (m.resources || []).map((r) => (typeof r === "string" ? r : r && r.name));
    return {
      name: m.name || "Stremio",
      catalogs: (m.catalogs || []).filter((c) => c.type === "movie" || c.type === "series"),
      // which ids this addon can give streams for (empty = any)
      streams: res.includes("stream"),
      subtitles: res.includes("subtitles"),
      idPrefixes: m.idPrefixes || [],
    };
  },

  // Stremio resource URLs: /{resource}/{type}/{id}[/{extra}].json; extra is a path segment, e.g. "search=hantu" or "genre=Anime&year=2025"
  videoCatalog(type, id, extra) {
    return json(`${this.base}/catalog/${encodeURIComponent(type)}/${encodeURIComponent(id)}${extra ? "/" + extra : ""}.json`).metas || [];
  },

  videoMeta(type, id) {
    return json(`${this.base}/meta/${encodeURIComponent(type)}/${encodeURIComponent(id)}.json`).meta || null;
  },

  // extra carries what helps the addon match the file: "filename=…&videoSize=…&videoHash=…"
  videoSubtitles(type, id, extra) {
    return json(`${this.base}/subtitles/${encodeURIComponent(type)}/${encodeURIComponent(id)}${extra ? "/" + extra : ""}.json`).subtitles || [];
  },

  // Fallback lives here (not the caller) because catalog metas carry no imdb_id. When a stream for
  // the catalog id (pm:post:…) 404s — the addon's index is incomplete — we discover the title's
  // IMDb id from the meta endpoint and retry; most titles resolve there (verified: Moana).
  videoStreams(type, id) {
    try {
      return streams(this.base, type, id);
    } catch (e) {
      const alt = discoverImdb(this.base, type, id);
      if (alt && alt !== id) return streams(this.base, type, alt);
      throw e;
    }
  },
});

function discoverImdb(base, type, id) {
  try {
    const m = json(`${base}/meta/${encodeURIComponent(type)}/${encodeURIComponent(id)}.json`).meta;
    return (m && (m.imdb_id || m._imdbId)) || null;
  } catch {
    return null; // meta is only a fallback source; report the original stream error
  }
}

function streams(base, type, id) {
  for (let i = 0; ; i++) {
    const r = fetch(`${base}/stream/${encodeURIComponent(type)}/${encodeURIComponent(id)}.json`);
    if (r.ok) return r.json().streams || [];
    if (r.status !== 404 || i > 0) {
      throw new Error(r.status === 404 ? "tidak ada sumber (server addon sedang sibuk, atau file belum terindeks)" : "HTTP " + r.status);
    }
    setTimeout(() => {}, 1500); // brief pause, then retry the same id once
  }
}

function json(url) {
  const r = fetch(url);
  if (!r.ok) throw new Error(r.status === 404 ? "server sibuk, coba lagi sebentar" : "HTTP " + r.status);
  return r.json() || {};
}
