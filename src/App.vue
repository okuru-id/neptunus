<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { computed, onMounted, onUnmounted, ref, watch, watchEffect } from "vue";
import CoverArt from "./lib/CoverArt.vue";
import { covers } from "./lib/covers";
import Icon from "./lib/Icon.vue";
import ConfirmDialog, { confirm } from "./lib/ConfirmDialog.vue";
import FileManager from "./lib/FileManager.vue";
import Extensions from "./lib/Extensions.vue";
import ExtCatalog from "./lib/ExtCatalog.vue";
import SongMenu from "./lib/SongMenu.vue";
import { downloadError, extName, has, unsupported, type ExtInfo } from "./lib/ext";
import { pasted, verifyError, waiting, withVerify } from "./lib/verify";
import { activity } from "./lib/activity";
import QualityTag from "./lib/QualityTag.vue";
import { loadQuality, longQuality, qualities, shortQuality } from "./lib/quality";
import TitleBar from "./lib/TitleBar.vue";
import type { IconName } from "./lib/icons";
import WavyProgress from "./lib/WavyProgress.vue";
import { applyTheme, DEFAULT_SEED, seedFromImage } from "./lib/theme";
import { ART_SHAPES, CLIP_SHAPES } from "./lib/shapes";

type Track = { path: string; title: string; artist: string; album: string; duration_secs: number };
type Playlist = { id: number; name: string };
type View = "catalog" | "songs" | "albums" | "artists" | "playlists" | "s3";

const NAV: { id: View; label: string; icon: IconName }[] = [
  { id: "catalog", label: "Katalog", icon: "catalog" },
  { id: "songs", label: "Lagu", icon: "note" },
  { id: "albums", label: "Album", icon: "album" },
  { id: "artists", label: "Artis", icon: "artist" },
  { id: "playlists", label: "Playlist", icon: "playlist" },
];

const rawLibrary = ref<Track[]>([]);
// A cloud song is hidden when a local one has the same title and artist (and about the same length).
const sameSong = (t: Track) => `${t.title.trim().toLowerCase()}\u0000${t.artist.trim().toLowerCase()}`;
const library = computed(() => {
  const local = new Map<string, number[]>();
  for (const t of rawLibrary.value) if (!t.path.startsWith("s3://")) local.set(sameSong(t), [...(local.get(sameSong(t)) ?? []), t.duration_secs]);
  return rawLibrary.value.filter(
    (t) => !t.path.startsWith("s3://") || !local.get(sameSong(t))?.some((d) => Math.abs(d - t.duration_secs) <= 3),
  );
});
const playlists = ref<Playlist[]>([]);
const loading = ref(false);

const view = ref<View>("catalog");
const query = ref("");
const group = ref<string | null>(null); // selected album/artist name
const plId = ref<number | null>(null);
const plTracks = ref<Track[]>([]);
const newPlaylist = ref("");

type S3Config = { endpoint: string; region: string; bucket: string; prefix: string; access_key: string; secret_key: string };
const s3 = ref<S3Config>({ endpoint: "", region: "us-east-1", bucket: "", prefix: "", access_key: "", secret_key: "" });
const s3Status = ref("");
const s3Busy = ref(false);
const s3Saved = ref(false); // a bucket is configured and saved
const s3Edit = ref(false);
const s3Root = computed(() => (!s3.value.prefix || s3.value.prefix.endsWith("/") ? s3.value.prefix : `${s3.value.prefix}/`));
const s3Where = computed(() => {
  const host = s3.value.endpoint ? s3.value.endpoint.replace(/^https?:\/\//, "") : `Amazon S3 · ${s3.value.region}`;
  return s3.value.prefix ? `${host} · /${s3.value.prefix}` : host;
});

const queue = ref<Track[]>([]);
const current = ref(-1);
const paused = ref(false);
const position = ref(0);
const volume = ref(0.8);
const shuffle = ref(false);
const sidebarOpen = ref(localStorage.getItem("sidebar") !== "0");
// narrow windows always get the mini rail
const narrowMq = matchMedia("(max-width: 900px)");
const narrow = ref(narrowMq.matches);
narrowMq.addEventListener("change", (e) => (narrow.value = e.matches));
const toggleSidebar = () => {
  sidebarOpen.value = !sidebarOpen.value;
  localStorage.setItem("sidebar", sidebarOpen.value ? "1" : "0");
};
const repeat = ref<"off" | "all" | "one">("off");
let seeking = false;
let preloaded = -1; // queue index already appended to the player for a gapless handoff

const showNow = ref(false);
const lyrics = ref<{ t: number | null; text: string }[]>([]);
const lyricsLoading = ref(false);
// manual LRCLIB search: shown when nothing was found, or on request to swap the lyrics
type LyricHit = { trackName: string; artistName: string; albumName: string | null; duration: number | null; syncedLyrics: string | null; plainLyrics: string | null };
const lyricSearchOpen = ref(false);
const lyricQuery = ref("");
const lyricHits = ref<LyricHit[] | null>(null);
const lyricBusy = ref(false);
const lyricErr = ref("");
const showLyricSearch = computed(() => lyricSearchOpen.value || (!lyrics.value.length && !lyricsLoading.value));

const now = computed<Track | undefined>(() => queue.value[current.value]);
const playing = computed(() => !!now.value && !paused.value);
const defs = ref<SVGSVGElement>();
const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
watchEffect(() => (playing.value ? defs.value?.unpauseAnimations() : defs.value?.pauseAnimations()));
const lyricIdx = computed(() =>
  lyrics.value[0]?.t == null ? -1 : lyrics.value.findLastIndex((l) => (l.t ?? Infinity) <= position.value + 0.2),
);
// karaoke fill of the current line: set once when the line starts (a running CSS animation can't follow
// the 500 ms position polls without jumping). LRC only stamps line starts, so the singing time is the gap to
// the next line, capped by a reading-pace estimate (~11 chars/s) so a following instrumental break doesn't
// stretch the fill. The delay is exact: positive when the line is picked a little early, negative when late.
const wipe = ref({ key: 0, dur: 4, delay: 0 });
function startWipe() {
  const i = lyricIdx.value, l = lyrics.value[i];
  if (!l || l.t == null) return;
  const gap = (lyrics.value[i + 1]?.t ?? l.t + 8) - l.t;
  const dur = Math.max(Math.min(gap - 0.15, 1 + l.text.length / 11), 0.5);
  wipe.value = { key: wipe.value.key + 1, dur, delay: l.t - position.value };
}
watch(lyricIdx, startWipe);
const q = computed(() => query.value.trim().toLowerCase());
const match = (t: Track) =>
  !q.value || [t.title, t.artist, t.album].some((s) => s.toLowerCase().includes(q.value));

const unknown = (s: string) => s || "Tidak diketahui";
const groupBy = (key: "album" | "artist") => {
  const m = new Map<string, Track[]>();
  for (const t of library.value.filter(match)) {
    const k = unknown(t[key]);
    m.set(k, [...(m.get(k) ?? []), t]);
  }
  return [...m.entries()].sort(([a], [b]) => a.localeCompare(b));
};
const albums = computed(() => groupBy("album"));
const artists = computed(() => groupBy("artist"));

// tracks shown in the main list, which also becomes the play queue
const shown = computed(() => {
  if (view.value === "songs") return library.value.filter(match);
  if (view.value === "playlists") return plTracks.value.filter(match);
  const g = (view.value === "albums" ? albums.value : artists.value).find(([k]) => k === group.value);
  return g ? g[1] : [];
});
const showingList = computed(() => view.value === "songs" || view.value === "playlists" || group.value !== null);
const currentPlaylist = computed(() => playlists.value.find((p) => p.id === plId.value));

const title = computed(() => {
  if (group.value !== null) return group.value;
  if (view.value === "playlists") return currentPlaylist.value?.name ?? "Playlist";
  if (view.value === "s3") return "Pengaturan";
  return NAV.find((n) => n.id === view.value)!.label;
});
const subtitle = computed(() => {
  if (view.value === "catalog") return `${library.value.length} lagu · ${albums.value.length} album · ${artists.value.length} artis`;
  if (view.value === "albums" && group.value === null) return `${albums.value.length} album`;
  if (view.value === "artists" && group.value === null) return `${artists.value.length} artis`;
  const total = shown.value.reduce((s, t) => s + t.duration_secs, 0);
  return `${shown.value.length} lagu · ${Math.round(total / 60)} menit`;
});

async function refreshPlaylists() {
  playlists.value = await invoke<Playlist[]>("list_playlists");
}
// Katalog: shelves that open the full album/artist/playlist pages (a Spotify catalog can join later)
const MORPH = (["cookie12", "clover4", "sunny8", "cookie9"] as const).flatMap((k) => [CLIP_SHAPES[k], CLIP_SHAPES[k]]).concat(CLIP_SHAPES.cookie12).join(";");
const SHELF = 20;
const PL_SHAPES = ["cookie9", "clover4", "sunny8", "cookie12"];
function openGroup(v: "albums" | "artists", name: string) {
  setView(v);
  group.value = name;
}
async function showPlaylist(id: number) {
  setView("playlists");
  await openPlaylist(id);
}
type Pending = { id: number; title: string; artist: string; album: string; duration_secs: number; uri: string };
// songs of an imported playlist that are not in the library yet; downloaded one by one on click
const plPending = ref<Pending[]>([]);
async function openPlaylist(id: number) {
  plId.value = id;
  [plTracks.value, plPending.value] = await Promise.all([
    invoke<Track[]>("playlist_tracks", { id }),
    invoke<Pending[]>("playlist_pending", { id }),
  ]);
}
const plNote = ref("");
async function importSpotify(url: string) {
  plNote.value = "Membaca playlist Spotify…";
  try {
    const r = await invoke<{ id: number; name: string; added: number; pending: number }>("spotify_import", { url });
    plNote.value = `“${r.name}”: ${r.added} lagu dari library, ${r.pending} belum diunduh (klik tombol unduh di baris lagunya).`;
    await refreshPlaylists();
    await openPlaylist(r.id);
  } catch (e) {
    plNote.value = `Gagal mengimpor: ${e}`;
  }
}
async function createPlaylist() {
  const name = newPlaylist.value.trim();
  if (!name) return;
  newPlaylist.value = "";
  if (/spotify\.com\/|^spotify:(playlist|album|artist|track):/.test(name)) return importSpotify(name);
  const id = await invoke<number>("create_playlist", { name });
  await refreshPlaylists();
  await openPlaylist(id);
}
async function deletePlaylist(id: number) {
  await invoke("delete_playlist", { id });
  if (plId.value === id) { plId.value = null; plTracks.value = []; plPending.value = []; }
  await refreshPlaylists();
}
// In-page menu rather than a native <select>: WebKitGTK shows that as a GTK popup whose pointer grab can
// get stuck under XWayland, leaving the whole window unclickable.
const addMenu = ref<string | null>(null);
const progress = ref<Record<string, number>>({}); // percent per downloading cloud song
const downloads = computed(() =>
  Object.entries(progress.value).map(([path, pct]) => ({ path, pct, title: rawLibrary.value.find((t) => t.path === path)?.title ?? "" })),
);
const isS3 = (path: string) => path.startsWith("s3://");
async function download(path: string) {
  progress.value = { ...progress.value, [path]: 0 };
  try {
    // the saved copy joins the library, which hides the cloud version of the same song
    rawLibrary.value = await invoke<Track[]>("s3_download", { path });
  } catch (e) {
    s3Status.value = `Gagal mengunduh: ${e}`;
  } finally {
    const { [path]: _, ...rest } = progress.value;
    progress.value = rest;
  }
}
async function addTo(id: number, path: string) {
  addMenu.value = null;
  await invoke("add_to_playlist", { id, path });
  if (plId.value === id) await openPlaylist(plId.value);
}
const lyricMenu = ref(false);
function closeMenu(e: PointerEvent) {
  if (!(e.target as HTMLElement).closest(".row-menu")) addMenu.value = null;
  if (!(e.target as HTMLElement).closest(".lyric-actions")) lyricMenu.value = false;
}
async function removeFrom(path: string) {
  if (plId.value === null) return;
  await invoke("remove_from_playlist", { id: plId.value, path });
  await openPlaylist(plId.value);
}

function setView(v: View) {
  view.value = v;
  group.value = null;
  // songs downloaded elsewhere (catalog) since take their place in the open playlist
  if (v === "playlists" && plId.value !== null) openPlaylist(plId.value);
}

type SettingsTab = "local" | "s3" | "ext";
const SETTINGS_TABS: { id: SettingsTab; label: string; icon: IconName }[] = [
  { id: "local", label: "Lokal", icon: "folder" },
  { id: "s3", label: "Cloud", icon: "cloud" },
  { id: "ext", label: "Ekstensi", icon: "extension" },
];
const settingsTab = ref<SettingsTab>("local");
function openSettings(tab: SettingsTab) {
  settingsTab.value = tab;
  setView("s3");
}
const setLibrary = (t: unknown[]) => (rawLibrary.value = t as Track[]);

// extensions (Settings › Ekstensi); metadata providers add their catalog to the Katalog page
const exts = ref<ExtInfo[]>([]);
const extCatalog = computed(() => exts.value.some((e) => e.enabled && has(e, "metadata_provider")));
// download extensions a song can be re-downloaded with
const downloaders = computed(() => exts.value.filter((e) => e.enabled && has(e, "download_provider") && !unsupported(e).length));
// re-download progress per library path (fed by `ext-download` events keyed by the path)
const redl = ref<Record<string, number>>({});
async function redownload(t: Track, id: string, quality: string | null = null) {
  redl.value = { ...redl.value, [t.path]: 0 };
  try {
    await invoke("ext_redownload", { path: t.path, id, quality });
    setLibrary(await invoke<Track[]>("load_library"));
    if (plId.value !== null) await openPlaylist(plId.value);
  } catch (e) {
    onDownloadFailed(t.title, id, e);
  } finally {
    const { [t.path]: _, ...rest } = redl.value;
    redl.value = rest;
  }
}
// pending playlist song -> progress percent while it downloads, and the extension doing it
const pdl = ref<Record<number, number>>({});
const pdlExt: Record<number, string> = {};
// short-lived notice at the bottom of the window (a failed download, ...)
const toast = ref("");
let toastTimer: ReturnType<typeof setTimeout> | undefined;
function notify(text: string) {
  toast.value = text;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toast.value = ""), 7000);
}
function onDownloadFailed(song: string, ext: string, err: unknown) {
  const e = exts.value.find((x) => x.id === ext);
  notify(downloadError(err, song, e ? extName(e) : ext));
}
// pending playlist song -> why its last download failed
const pfail = ref<Record<number, string>>({});
async function downloadPending(p: Pending, ext: string, quality: string | null) {
  const { [p.id]: _, ...ok } = pfail.value;
  pfail.value = ok;
  pdl.value = { ...pdl.value, [p.id]: 0 };
  pdlExt[p.id] = ext;
  activity[ext] = "";
  try {
    // the song as the catalog knows it (full metadata, cover, album), so it downloads the same way
    const meta = exts.value.find((e) => e.enabled && e.id === "spotify-web");
    const sid = p.uri.split(":")[2];
    const track = meta && sid
      ? await invoke<Record<string, unknown> | null>("ext_call", { id: meta.id, method: "getTrack", args: [sid] })
          .then((t) => (t ? { ...t, provider_id: meta.id } : null)).catch(() => null)
      : null;
    await withVerify(ext, () => invoke("pending_download", { id: p.id, ext, quality, track }), () => (pdl.value = { ...pdl.value, [p.id]: 0 }));
    setLibrary(await invoke<Track[]>("load_library"));
    if (plId.value !== null) await openPlaylist(plId.value);
  } catch (e) {
    // like the catalog: no dialog, the row's button turns into "try again" with the reason as its tooltip
    pfail.value = { ...pfail.value, [p.id]: String(e) };
    onDownloadFailed(p.title, ext, e);
  } finally {
    const { [p.id]: _, ...rest } = pdl.value;
    pdl.value = rest;
  }
}
async function removePending(p: Pending) {
  await invoke("pending_remove", { id: p.id });
  if (plId.value !== null) await openPlaylist(plId.value);
}
async function removeSong(t: Track) {
  const s3 = isS3(t.path);
  const ok = await confirm({
    title: `Hapus “${t.title}”?`,
    message: s3 ? "File dihapus dari bucket S3 dan dikeluarkan dari library." : "File dipindahkan ke Tempat Sampah dan dikeluarkan dari library.",
    confirm: "Hapus", icon: "trash", danger: true,
  });
  if (!ok) return;
  try {
    setLibrary(s3
      ? await invoke<Track[]>("s3_delete", { keys: [t.path.replace(/^s3:\/\/[^/]+\//, "")] })
      : await invoke<Track[]>("local_delete", { keys: [t.path] }));
    if (plId.value !== null) await openPlaylist(plId.value);
  } catch (e) {
    await confirm({ title: "Gagal menghapus", message: String(e), confirm: "OK", icon: "trash", danger: true });
  }
}
async function onDownloaded() {
  setLibrary(await invoke<Track[]>("load_library"));
}

const folders = ref<string[]>([]);
const localFiles = ref<{ reload: () => void } | null>(null);
const curFolder = ref<string | null>(null);
const baseName = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;
async function refreshFolders() {
  folders.value = await invoke<string[]>("list_folders");
  if (!curFolder.value || !folders.value.includes(curFolder.value)) curFolder.value = folders.value[0] ?? null;
}
async function scan(dir: string) {
  loading.value = true;
  try {
    rawLibrary.value = await invoke<Track[]>("scan_folder", { path: dir });
    await refreshFolders();
    localFiles.value?.reload();
  } finally {
    loading.value = false;
  }
}
async function pickFolder() {
  const dir = await open({ directory: true });
  if (!dir) return;
  curFolder.value = dir;
  await scan(dir);
  if (view.value !== "s3") setView("songs");
}
const rescan = (dir: string) => scan(dir);
async function forgetFolder(dir: string) {
  const ok = await confirm({
    title: `Keluarkan "${baseName(dir)}"?`,
    message: "Folder dan lagunya dikeluarkan dari library. File di komputer tidak dihapus.",
    confirm: "Keluarkan",
    icon: "folder",
  });
  if (!ok) return;
  rawLibrary.value = await invoke<Track[]>("remove_folder", { path: dir });
  await refreshFolders();
}

async function s3Save() {
  await invoke("s3_save_config", { config: s3.value });
  flash("Tersimpan.");
  s3Saved.value = !!s3.value.bucket;
  s3Edit.value = false;
}
// success messages go away by themselves; errors stay until closed or replaced
function flash(msg: string) {
  s3Status.value = msg;
  setTimeout(() => s3Status.value === msg && (s3Status.value = ""), 4000);
}
async function s3Cancel() {
  s3.value = await invoke<S3Config>("s3_get_config");
  s3Edit.value = false;
  s3Status.value = "";
}
async function s3Sync() {
  s3Busy.value = true;
  s3Status.value = "Menghubungkan…";
  try {
    await s3Save();
    rawLibrary.value = await invoke<Track[]>("s3_sync");
    flash("Selesai. Lagu sudah masuk ke library.");
  } catch (e) {
    s3Status.value = `Gagal: ${e}`;
  } finally {
    s3Busy.value = false;
  }
}

// File to decode: the local copy of a cloud song when there is one, so it is never streamed needlessly.
function source(t: Track): string {
  if (!isS3(t.path)) return t.path;
  const key = sameSong(t);
  const twin = rawLibrary.value.find((l) => !isS3(l.path) && sameSong(l) === key && Math.abs(l.duration_secs - t.duration_secs) <= 3);
  return twin?.path ?? t.path;
}

async function playAt(list: Track[], i: number) {
  if (!list[i]) return;
  queue.value = list;
  current.value = i;
  preloaded = -1;
  position.value = 0;
  paused.value = false;
  await invoke("play", { path: source(list[i]), volume: volume.value });
}

// the audio lives in the backend and survives a webview reload, so keep what the page needs to show it
const SESSION = "neptunus.nowPlaying";
function saveNow() {
  try {
    if (current.value < 0) sessionStorage.removeItem(SESSION);
    else sessionStorage.setItem(SESSION, JSON.stringify({ queue: queue.value, current: current.value, preloaded }));
  } catch {}
}
watch([queue, current], saveNow);
async function restorePlayer() {
  const st = await invoke<{ position_secs: number; paused: boolean; finished: boolean } | null>("status");
  if (!st || st.finished) return;
  try {
    const s = JSON.parse(sessionStorage.getItem(SESSION) ?? "null");
    if (!s?.queue?.[s.current]) return;
    queue.value = s.queue;
    current.value = s.current;
    preloaded = s.preloaded ?? -1;
    position.value = st.position_secs;
    paused.value = st.paused;
  } catch {}
}

function nextIndex(auto: boolean): number | null {
  if (!queue.value.length) return null;
  if (auto && repeat.value === "one") return current.value;
  if (shuffle.value) return Math.floor(Math.random() * queue.value.length);
  if (current.value + 1 < queue.value.length) return current.value + 1;
  return repeat.value === "all" ? 0 : null;
}

function next(auto = false) {
  const i = nextIndex(auto);
  if (i === null) current.value = -1;
  else playAt(queue.value, i);
}

function prev() {
  if (position.value > 3 || current.value <= 0) return playAt(queue.value, Math.max(current.value, 0));
  playAt(queue.value, current.value - 1);
}

async function togglePause() {
  if (current.value < 0) return playAt(shown.value, 0);
  paused.value = !paused.value;
  await invoke("set_paused", { paused: paused.value });
}

function playShuffled() {
  if (!shown.value.length) return;
  shuffle.value = true;
  playAt(shown.value, Math.floor(Math.random() * shown.value.length));
}

const cycleRepeat = () => (repeat.value = repeat.value === "off" ? "all" : repeat.value === "all" ? "one" : "off");

async function poll() {
  if (current.value < 0) return;
  const st = await invoke<{ position_secs: number; paused: boolean; finished: boolean; queued: number } | null>("status");
  if (!st) return;
  if (st.finished) {
    if (preloaded >= 0) current.value = preloaded;
    preloaded = -1;
    return next(true);
  }
  if (preloaded >= 0 && st.queued < 2) {
    // the preloaded track took over seamlessly
    current.value = preloaded;
    preloaded = -1;
  }
  if (!seeking) position.value = st.position_secs;
  if (preloaded < 0 && st.queued === 1 && now.value && now.value.duration_secs - st.position_secs < 10) {
    const i = nextIndex(true);
    if (i !== null) {
      preloaded = i;
      saveNow();
      invoke("queue_next", { path: source(queue.value[i]) }).catch(() => (preloaded = -1));
    }
  }
}

function parseLyrics(raw: string | null) {
  if (!raw) return [];
  const out: { t: number | null; text: string }[] = [];
  for (const line of raw.split(/\r?\n/)) {
    const stamps = [...line.matchAll(/\[(\d+):(\d+(?:\.\d+)?)\]/g)];
    const text = line.replace(/\[[^\]]*\]|<\d+:\d+(?:\.\d+)?>/g, "").replace(/\s+/g, " ").trim();
    if (stamps.length) for (const m of stamps) out.push({ t: +m[1] * 60 + +m[2], text });
    else if (!/^\[\w+:/.test(line)) out.push({ t: null, text: line.trim() });
  }
  return out.some((l) => l.t != null)
    ? out.filter((l) => l.t != null).sort((a, b) => a.t! - b.t!)
    : out;
}

async function loadLyrics(refresh = false) {
  const n = now.value;
  if (!n) return;
  lyricsLoading.value = true;
  const r = await invoke<string | null>("get_lyrics", { path: n.path, title: n.title, artist: n.artist, album: n.album, duration: n.duration_secs, refresh });
  if (now.value?.path !== n.path) return;
  lyrics.value = parseLyrics(r);
  lyricsLoading.value = false;
  if (refresh && !lyrics.value.length) lyricErr.value = "Masih tidak ditemukan. Coba cari manual dengan kata kunci lain.";
}
async function searchLyrics() {
  const q = lyricQuery.value.trim();
  if (!q) return;
  lyricBusy.value = true;
  lyricErr.value = "";
  try {
    lyricHits.value = await invoke<LyricHit[]>("search_lyrics", { query: q });
  } catch (e) {
    lyricErr.value = `Gagal mencari: ${e}`;
  } finally {
    lyricBusy.value = false;
  }
}
async function pickLyrics(h: LyricHit) {
  const n = now.value;
  const text = h.syncedLyrics ?? h.plainLyrics;
  if (!n || !text) return;
  await invoke("save_lyrics", { title: n.title, artist: n.artist, album: n.album, text });
  lyrics.value = parseLyrics(text);
  lyricSearchOpen.value = false;
}

// Forgets the lyrics for this song: stored as "not found", so the wrong ones are not fetched again.
async function clearLyrics() {
  const n = now.value;
  if (!n) return;
  await invoke("save_lyrics", { title: n.title, artist: n.artist, album: n.album, text: "" });
  lyrics.value = [];
  lyricSearchOpen.value = false;
}

// lyrics + dynamic colour follow the playing track
const coverTried = new Set<string>();
watch(() => now.value?.path, (path) => {
  if (path) loadQuality(path);
  lyrics.value = [];
  lyricSearchOpen.value = false;
  lyricHits.value = null;
  lyricErr.value = "";
  if (!path) return;
  const n = now.value!;
  // missing artist/album ("Tidak diketahui"): look it up online once per song per session
  if ((!n.artist || !n.album) && !coverTried.has("meta:" + path)) {
    coverTried.add("meta:" + path);
    invoke<Track | null>("find_meta", { path }).then((t) => {
      if (t) for (const x of [...rawLibrary.value, ...plTracks.value, ...queue.value]) if (x.path === path) Object.assign(x, t);
    }).catch(() => {});
  }
  lyricQuery.value = [n.title, n.artist].filter(Boolean).join(" ");
  loadLyrics();
  invoke<string | null>("get_cover", { path }).then(async (c) => {
    // no embedded cover: look it up online once per song per session
    if (!c && !coverTried.has(path)) {
      coverTried.add(path);
      c = await invoke<string | null>("find_cover", { path }).catch(() => null);
    }
    covers[path] = c;
    const seed = c ? await seedFromImage(c).catch(() => DEFAULT_SEED) : DEFAULT_SEED;
    if (now.value?.path === path) applyTheme(seed);
  });
});

watch([lyricIdx, showNow], () => {
  if (lyricIdx.value >= 0 && showNow.value)
    document.querySelector(".lyrics .cur")?.scrollIntoView({ block: "center", behavior: "smooth" });
}, { flush: "post" });

// OS media keys / MPRIS / SMTC
watch(now, (n) => {
  if (n)
    invoke("media_metadata", {
      path: n.path, title: n.title, artist: n.artist, album: n.album, durationSecs: n.duration_secs,
    });
});
watch([current, paused], () => {
  invoke("media_playback", { playing: current.value < 0 ? null : !paused.value, position: position.value });
}, { immediate: true });

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape" && addMenu.value) return void (addMenu.value = null);
  const el = e.target as HTMLElement;
  if (el.closest("input, select, textarea, button")) return;
  if (e.code === "Space") { e.preventDefault(); togglePause(); }
  else if (e.key === "Escape") showNow.value = false;
}

let pollId = 0;
const subs: Promise<UnlistenFn>[] = [];
onMounted(async () => {
  window.addEventListener("keydown", onKey);
  window.addEventListener("pointerdown", closeMenu);
  pollId = setInterval(poll, 500);
  invoke<ExtInfo[]>("ext_list").then((l) => (exts.value = l));
  subs.push(
    listen<{ key: string; pct: number }>("ext-download", ({ payload: { key, pct } }) => {
      if (key in redl.value) redl.value = { ...redl.value, [key]: pct };
      const pid = key.startsWith("pending:") ? Number(key.slice(8)) : NaN;
      if (pid in pdl.value) pdl.value = { ...pdl.value, [pid]: Math.round(pct) };
    }),
    listen<[string, number, number]>("s3-download", ({ payload: [path, done, total] }) => {
      if (path in progress.value) progress.value = { ...progress.value, [path]: total ? Math.round((done / total) * 100) : 0 };
    }),
    listen<[number, number]>("s3-progress", async ({ payload: [i, n] }) => {
      s3Status.value = `Membaca ${i} dari ${n} lagu…`;
      // each batch is saved before this event, so the songs read so far can show up right away
      rawLibrary.value = await invoke<Track[]>("load_library");
    }),
    listen<string>("media-key", ({ payload }) => {
      if (payload === "next") next();
      else if (payload === "prev") prev();
      else if (payload === "toggle" || (payload === "play") === paused.value) togglePause();
    }),
    listen<number>("media-seek", async ({ payload }) => {
      await invoke("seek", { secs: payload });
      position.value = payload;
    }),
  );
  applyTheme(DEFAULT_SEED);
  await restorePlayer();
  rawLibrary.value = await invoke<Track[]>("load_library");
  await refreshPlaylists();
  s3.value = await invoke<S3Config>("s3_get_config");
  s3Saved.value = !!s3.value.bucket;
  await refreshFolders();
});
onUnmounted(() => {
  window.removeEventListener("keydown", onKey);
  window.removeEventListener("pointerdown", closeMenu);
  clearInterval(pollId);
  subs.forEach((p) => p.then((off) => off()));
});

function scrub(v: number) {
  seeking = true;
  position.value = v;
}
async function seekTo(v: number) {
  await invoke("seek", { secs: v });
  position.value = v;
  seeking = false;
  startWipe(); // a seek inside the same line must restart the fill from the new spot
}

const fmt = (s: number) => {
  s = Math.floor(s);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
};
const shapeOf = (i: number) => ART_SHAPES[i % ART_SHAPES.length];
const repeatLabel = computed(() => (repeat.value === "off" ? "mati" : repeat.value === "all" ? "semua" : "satu lagu"));
</script>

<template>
  <svg ref="defs" class="defs" aria-hidden="true">
    <defs>
      <clipPath v-for="(d, id) in CLIP_SHAPES" :key="id" :id="`shape-${id}`" clipPathUnits="objectBoundingBox"><path :d="d" /></clipPath>
      <!-- the shape itself spins: rotating a clipped element with CSS leaves stray edge lines in WebKitGTK -->
      <clipPath id="shape-now" clipPathUnits="objectBoundingBox">
        <path :d="CLIP_SHAPES.cookie12">
          <animateTransform v-if="!reduceMotion" attributeName="transform" type="rotate" from="0 0.5 0.5" to="360 0.5 0.5" dur="40s" repeatCount="indefinite" />
          <!-- every shape has the same points, so the outline can morph between them: hold, morph, hold, … -->
          <animate v-if="!reduceMotion" attributeName="d" :values="MORPH" dur="24s" repeatCount="indefinite"
            calcMode="spline" keyTimes="0;0.125;0.25;0.375;0.5;0.625;0.75;0.875;1" :keySplines="Array(8).fill('0.4 0 0.2 1').join(';')" />
        </path>
      </clipPath>
    </defs>
  </svg>

  <ConfirmDialog />
  <TitleBar @settings="openSettings('local')" />

  <div class="app" :class="{ collapsed: !sidebarOpen || narrow }">
    <nav class="drawer" :class="{ 'has-player': !!now }" aria-label="Navigasi">
      <button class="fab" :class="{ sel: view === 'catalog' }" title="Katalog" @click="setView('catalog')" :aria-current="view === 'catalog' ? 'page' : undefined">
        <Icon name="catalog" /><span class="lbl">Katalog</span>
      </button>
      <div class="dest">
        <button v-for="n in NAV.slice(1)" :key="n.id" class="dest-item" :title="n.label" :class="{ sel: view === n.id }" @click="setView(n.id)" :aria-current="view === n.id ? 'page' : undefined">
          <Icon :name="n.icon" />
          <span class="lbl">{{ n.label }}</span>
          <span v-if="n.id === 'playlists' && playlists.length" class="badge">{{ playlists.length }}</span>
        </button>
      </div>
      <button v-if="!narrow" class="icon-btn tonal collapse-btn" @click="toggleSidebar" :title="sidebarOpen ? 'Tutup sidebar' : 'Buka sidebar'"><Icon :name="sidebarOpen ? 'panelClose' : 'panelOpen'" /></button>
    </nav>

    <main :class="{ 'has-player': !!now }">
      <header class="top">
        <div class="heading">
          <button v-if="group !== null" class="icon-btn tonal" @click="group = null" title="Kembali"><Icon name="back" /></button>
          <div>
            <h1>{{ title }}</h1>
            <p v-if="library.length && view !== 's3'" class="sub">{{ subtitle }}</p>
          </div>
        </div>
        <label v-if="(library.length || (view === 'catalog' && extCatalog)) && view !== 's3'" class="search">
          <Icon name="search" />
          <input placeholder="Cari lagu, artis, album" v-model="query" />
          <button v-if="query" class="icon-btn" @click="query = ''" title="Hapus pencarian"><Icon name="close" :size="20" /></button>
        </label>
        <div v-if="showingList && (view !== 'playlists' || plId !== null) && shown.length" class="btn-group actions">
          <button class="btn filled big" @click="playAt(shown, 0)"><Icon name="play" :size="28" />Putar</button>
          <button class="btn tonal big" @click="playShuffled"><Icon name="shuffle" />Acak</button>
        </div>
        <div v-if="view === 's3'" class="tabs" role="tablist">
          <button v-for="t in SETTINGS_TABS" :key="t.id" class="tab" :class="{ sel: settingsTab === t.id }" role="tab"
            :aria-selected="settingsTab === t.id" @click="settingsTab = t.id"><span class="tab-c"><Icon :name="t.icon" :size="24" />{{ t.label }}</span></button>
        </div>
      </header>

      <p v-if="loading" class="scanning"><span class="spinner" :style="{ clipPath: 'url(#shape-cookie12)' }"></span>Memindai folder musik…</p>

      <section v-if="!library.length && view !== 's3' && !(view === 'catalog' && extCatalog)" class="empty">
        <div class="empty-mark" :style="{ clipPath: 'url(#shape-cookie12)' }"><Icon name="note" :size="72" /></div>
        <h2>Belum ada musik</h2>
        <p>Pilih folder berisi lagu di komputer ini, atau sinkronkan dari bucket S3.</p>
        <div class="btn-group">
          <button class="btn filled" @click="pickFolder"><Icon name="addFolder" />Pilih folder musik</button>
          <button class="btn tonal" @click="openSettings('s3')"><Icon name="cloud" />Hubungkan S3</button>
        </div>
      </section>
      <div v-else-if="view === 's3'" class="cloud">
        <template v-if="settingsTab === 'local'">
          <section class="panel conn">
            <div class="conn-head">
              <span class="conn-mark" :style="{ clipPath: 'url(#shape-cookie9)' }"><Icon name="folder" :size="28" /></span>
              <div class="conn-id">
                <strong>Folder musik</strong>
                <span>{{ folders.length ? `${folders.length} folder di library` : "Tambahkan folder berisi lagu di komputer ini." }}</span>
              </div>
              <button class="btn filled" @click="pickFolder" :disabled="loading"><Icon name="addFolder" />{{ loading ? "Memindai…" : "Tambah folder" }}</button>
            </div>
            <div v-if="folders.length" class="chips folder-chips" role="list">
              <span v-for="f in folders" :key="f" class="chip" :class="{ sel: curFolder === f }" role="listitem">
                <button @click="curFolder = f" :title="f"><Icon name="folder" :size="18" />{{ baseName(f) }}</button>
                <template v-if="curFolder === f">
                  <button class="chip-x" title="Pindai ulang" :disabled="loading" @click="rescan(f)"><Icon name="sync" :size="18" /></button>
                  <button class="chip-x" title="Keluarkan dari library" @click="forgetFolder(f)"><Icon name="close" :size="18" /></button>
                </template>
              </span>
            </div>
          </section>
          <FileManager v-if="curFolder" ref="localFiles" :key="curFolder" kind="local" :root="`${curFolder.replace(/\/+$/, '')}/`" :label="baseName(curFolder)"
            @library="setLibrary" />
        </template>
        <Extensions v-else-if="settingsTab === 'ext'" @changed="exts = $event" />
        <template v-else>
        <section class="panel conn">
          <div class="conn-head">
            <span class="conn-mark" :style="{ clipPath: 'url(#shape-cookie9)' }"><Icon name="cloud" :size="28" /></span>
            <div class="conn-id">
              <strong>{{ s3Saved ? s3.bucket : "Belum terhubung" }}</strong>
              <span>{{ s3Saved ? s3Where : "Hubungkan bucket S3 (AWS, MinIO, R2, …) untuk men-stream dan mengelola musik di cloud." }}</span>
            </div>
            <div v-if="s3Saved && !s3Edit" class="btn-group">
              <button class="btn tonal" @click="s3Sync" :disabled="s3Busy"><Icon name="sync" />{{ s3Busy ? "Menyinkronkan…" : "Sinkronkan" }}</button>
              <button class="icon-btn tonal conn-edit" title="Ubah koneksi" aria-label="Ubah koneksi" @click="s3Edit = true"><Icon name="edit" /></button>
            </div>
          </div>
          <form v-if="!s3Saved || s3Edit" class="conn-form" @submit.prevent="s3Sync">
            <p class="hint">Lagu tidak diunduh: hanya metadata yang dibaca, lalu lagu di-stream langsung dari bucket saat diputar.</p>
            <div class="fields">
              <label class="field wide">Endpoint<input v-model="s3.endpoint" placeholder="Kosongkan untuk AWS, mis. https://minio.contoh.com" /></label>
              <label class="field">Region<input v-model="s3.region" /></label>
              <label class="field">Bucket<input v-model="s3.bucket" required /></label>
              <label class="field wide">Prefix<input v-model="s3.prefix" placeholder="Opsional, mis. music/" /></label>
              <label class="field">Access key<input v-model="s3.access_key" required autocomplete="off" /></label>
              <label class="field">Secret key<input type="password" v-model="s3.secret_key" required autocomplete="off" /></label>
            </div>
            <div class="btn-group">
              <button class="btn filled" :disabled="s3Busy"><Icon name="sync" />{{ s3Busy ? "Menyinkronkan…" : "Simpan & sinkronkan" }}</button>
              <button type="button" class="btn tonal" @click="s3Save" :disabled="s3Busy"><Icon name="save" />Simpan saja</button>
              <button v-if="s3Saved" type="button" class="btn outlined" @click="s3Cancel"><Icon name="close" />Batal</button>
            </div>
          </form>
          <Transition name="fade">
            <p v-if="s3Status" class="status" :class="{ err: s3Status.startsWith('Gagal') }" role="status">
              <span>{{ s3Status }}</span>
              <button v-if="!s3Busy" class="icon-btn small" title="Tutup" @click="s3Status = ''"><Icon name="close" :size="18" /></button>
            </p>
          </Transition>
        </section>
        <FileManager v-if="s3Saved && !s3Edit" kind="s3" :root="s3Root" :label="s3.bucket" @library="setLibrary" />
        </template>
      </div>
      <template v-else>
        <div v-if="view === 'playlists'" class="chips" role="list">
          <span v-for="p in playlists" :key="p.id" class="chip" :class="{ sel: plId === p.id }" role="listitem">
            <button @click="openPlaylist(p.id)">{{ p.name }}</button>
            <button v-if="plId === p.id" class="chip-x" :title="`Hapus playlist ${p.name}`" @click="deletePlaylist(p.id)"><Icon name="close" :size="18" /></button>
          </span>
          <form class="chip new" @submit.prevent="createPlaylist">
            <Icon name="add" :size="18" />
            <input placeholder="Playlist baru / link Spotify" v-model="newPlaylist" aria-label="Nama playlist baru" />
          </form>
        </div>
        <div v-if="view === 'playlists' && waiting" class="pl-note verify">
          Selesaikan verifikasi di jendela yang terbuka; unduhan lanjut otomatis. Kalau jendela tidak menutup sendiri, tempel tautan
          <code>spotiflac://session-grant?…</code> di sini:
          <input v-model="pasted" placeholder="spotiflac://session-grant?grant=…&state=…" spellcheck="false" />
        </div>
        <p v-else-if="view === 'playlists' && verifyError" class="pl-note">{{ verifyError }}</p>
        <p v-if="view === 'playlists' && plNote" class="pl-note" role="status">
          {{ plNote }}
          <button class="icon-btn small" title="Tutup" @click="plNote = ''"><Icon name="close" :size="18" /></button>
        </p>

        <div v-if="view === 'catalog'" class="catalog">
          <ExtCatalog :exts="exts" :library="rawLibrary" :query="query" :now-path="now?.path" :playing="playing" :redl="redl"
            @downloaded="onDownloaded" @failed="onDownloadFailed" @play="(t) => playAt([t], 0)" @toggle="togglePause" @redownload="redownload" @remove="removeSong" />
          <section v-if="albums.length" class="shelf">
            <div class="shelf-head"><h2>Album</h2><button class="more" @click="setView('albums')">Lihat semua</button></div>
            <div class="shelf-row">
              <button v-for="([name, list], i) in albums.slice(0, SHELF)" :key="name" class="tile" @click="openGroup('albums', name)">
                <CoverArt :path="list[0].path" :shape="shapeOf(i)" />
                <strong>{{ name }}</strong>
                <span>{{ unknown(list[0].artist) }}</span>
              </button>
            </div>
          </section>
          <section v-if="artists.length" class="shelf">
            <div class="shelf-head"><h2>Artis</h2><button class="more" @click="setView('artists')">Lihat semua</button></div>
            <div class="shelf-row artists">
              <button v-for="[name, list] in artists.slice(0, SHELF)" :key="name" class="tile" @click="openGroup('artists', name)">
                <CoverArt :path="list[0].path" shape="circle" />
                <strong>{{ name }}</strong>
                <span>{{ list.length }} lagu</span>
              </button>
            </div>
          </section>
          <section v-if="playlists.length && !q" class="shelf">
            <div class="shelf-head"><h2>Playlist</h2><button class="more" @click="setView('playlists')">Lihat semua</button></div>
            <div class="shelf-row">
              <button v-for="(p, i) in playlists.slice(0, SHELF)" :key="p.id" class="tile" @click="showPlaylist(p.id)">
                <span class="pl-art" :style="{ clipPath: `url(#shape-${PL_SHAPES[i % PL_SHAPES.length]})` }"><Icon name="playlist" :size="56" /></span>
                <strong>{{ p.name }}</strong>
                <span>Playlist</span>
              </button>
            </div>
          </section>
          <p v-if="library.length && !albums.length && !artists.length" class="none">Tidak ada yang cocok dengan “{{ query }}”.</p>
        </div>
        <div v-else-if="(view === 'albums' || view === 'artists') && group === null" class="grid" :class="{ artists: view === 'artists' }">
          <button v-for="([name, list], i) in view === 'albums' ? albums : artists" :key="name" class="tile" @click="group = name">
            <CoverArt :path="list[0].path" :shape="view === 'artists' ? 'circle' : shapeOf(i)" />
            <strong>{{ name }}</strong>
            <span>{{ view === "albums" ? unknown(list[0].artist) : `${list.length} lagu` }}</span>
          </button>
          <p v-if="!(view === 'albums' ? albums : artists).length" class="none">Tidak ada yang cocok dengan “{{ query }}”.</p>
        </div>
        <ul v-else-if="showingList" class="songs">
          <li v-for="(t, i) in shown" :key="t.path" :class="{ active: now?.path === t.path }">
            <button class="row" @dblclick="playAt(shown, i)" @keydown.enter="playAt(shown, i)" title="Klik dua kali untuk memutar">
              <span class="art-wrap">
                <CoverArt :path="t.path" />
                <span v-if="now?.path === t.path" class="art-overlay">
                  <span class="eq" :class="{ still: paused }" aria-label="Sedang diputar"><i></i><i></i><i></i></span>
                </span>
              </span>
              <span class="meta">
                <strong><Icon v-if="isS3(t.path)" name="cloud" :size="16" class="cloud-mark" title="Di-stream dari S3" />{{ t.title }}</strong>
                <span>{{ unknown(t.artist) }} · {{ unknown(t.album) }}</span>
              </span>
            </button>
            <button class="icon-btn row-act" title="Putar" :aria-label="`Putar ${t.title}`" @click="playAt(shown, i)">
              <Icon name="play" :size="22" />
            </button>
            <button v-if="isS3(t.path)" class="icon-btn row-act dl" :disabled="t.path in progress"
              :title="t.path in progress ? `Mengunduh ${progress[t.path]}%` : 'Unduh ke folder Musik'"
              :aria-label="`Unduh ${t.title}`" @click="download(t.path)">
              <span v-if="t.path in progress" class="pct">{{ progress[t.path] }}%</span>
              <Icon v-else name="download" :size="20" />
            </button>
            <button v-if="view === 'playlists'" class="icon-btn row-act" title="Hapus dari playlist" @click="removeFrom(t.path)"><Icon name="close" :size="20" /></button>
            <div v-else-if="playlists.length" class="row-menu" :class="{ open: addMenu === t.path }">
              <button class="icon-btn row-act" title="Tambah ke playlist" :aria-label="`Tambah ${t.title} ke playlist`"
                :aria-expanded="addMenu === t.path" @click="addMenu = addMenu === t.path ? null : t.path">
                <Icon name="playlistAdd" :size="20" />
              </button>
              <div v-if="addMenu === t.path" class="pl-menu" role="menu">
                <p>Tambah ke playlist</p>
                <button v-for="p in playlists" :key="p.id" role="menuitem" @click="addTo(p.id, t.path)">{{ p.name }}</button>
              </div>
            </div>
            <SongMenu class="row-act" :class="{ busy: t.path in redl }" :title="t.title" :downloaders="isS3(t.path) ? [] : downloaders"
              :busy="redl[t.path] ?? null" @redownload="(id, q) => redownload(t, id, q)" @remove="removeSong(t)" />
            <QualityTag :path="t.path" />
            <span class="dur">{{ fmt(t.duration_secs) }}</span>
          </li>
          <template v-if="view === 'playlists'">
            <li v-for="p in plPending" :key="`p${p.id}`" class="pending">
              <div class="row">
                <span class="art-wrap"><span class="pend-art"><Icon name="note" :size="24" /></span></span>
                <span class="meta">
                  <strong>{{ p.title }}</strong>
                  <span v-if="p.id in pdl" class="live">{{ activity[pdlExt[p.id]] || "Memulai…" }}</span>
                  <span v-else>{{ unknown(p.artist) }}<template v-if="p.album"> · {{ p.album }}</template> · belum diunduh</span>
                </span>
              </div>
              <SongMenu class="row-act" :class="{ busy: p.id in pdl || p.id in pfail }" download :title="p.title" :downloaders="downloaders"
                :busy="pdl[p.id] ?? null" :failed="pfail[p.id]"
                @redownload="(id, q) => downloadPending(p, id, q)" />
              <button class="icon-btn row-act" title="Hapus dari playlist" :disabled="p.id in pdl" @click="removePending(p)"><Icon name="close" :size="20" /></button>
              <span class="dur">{{ fmt(p.duration_secs) }}</span>
            </li>
          </template>
          <p v-if="!shown.length && !(view === 'playlists' && plPending.length)" class="none">
            <template v-if="view === 'playlists' && plId === null">
              {{ playlists.length ? "Pilih playlist di atas." : "Buat playlist pertama dengan mengetik namanya di atas." }}
            </template>
            <template v-else-if="view === 'playlists' && !query">Playlist ini masih kosong. Tambahkan lagu dari tab Lagu.</template>
            <template v-else>Tidak ada yang cocok dengan “{{ query }}”.</template>
          </p>
        </ul>
      </template>
    </main>
  </div>

  <Transition name="now">
  <section v-if="now && showNow" class="now" aria-label="Sedang diputar">
    <button class="icon-btn tonal close" @click="showNow = false" title="Tutup (Esc)"><Icon name="collapse" :size="28" /></button>
    <div v-if="lyrics.length && !lyricSearchOpen" class="lyric-actions" @keydown.esc.stop="lyricMenu = false">
      <button class="icon-btn tonal" title="Opsi lirik" aria-haspopup="menu" :aria-expanded="lyricMenu" @click="lyricMenu = !lyricMenu"><Icon name="moreVert" /></button>
      <Transition name="menu">
        <div v-if="lyricMenu" class="pl-menu" role="menu">
          <button role="menuitem" @click="lyricMenu = false; lyricSearchOpen = true"><Icon name="search" :size="20" />Cari lirik lain</button>
          <button role="menuitem" @click="lyricMenu = false; clearLyrics()"><Icon name="trash" :size="20" />Hapus lirik</button>
        </div>
      </Transition>
    </div>
    <div class="stage">
      <div class="spin" :style="{ clipPath: 'url(#shape-now)' }">
        <img v-if="covers[now.path]" :src="covers[now.path]!" :alt="`Sampul ${now.album}`" />
        <Icon v-else name="note" :size="120" />
      </div>
      <h2>{{ now.title }}</h2>
      <p>{{ unknown(now.artist) }} · {{ unknown(now.album) }}</p>
      <p v-if="qualities[now.path]" class="now-q">{{ longQuality(qualities[now.path]!) }}</p>
    </div>
    <div v-if="showLyricSearch" class="lyric-search">
      <div class="ls-head">
        <Icon name="lyrics" :size="32" />
        <div>
          <strong>{{ lyrics.length ? "Cari lirik lain" : "Lirik tidak tersedia" }}</strong>
          <span>Cari di LRCLIB, lalu pilih lirik yang cocok. Pilihan Anda disimpan untuk lagu ini.</span>
        </div>
        <button v-if="lyrics.length" class="icon-btn" title="Kembali ke lirik" @click="lyricSearchOpen = false"><Icon name="close" /></button>
      </div>
      <form class="ls-bar" @submit.prevent="searchLyrics">
        <label class="search">
          <Icon name="search" />
          <input v-model="lyricQuery" placeholder="Judul dan artis" aria-label="Cari lirik" />
        </label>
        <button class="btn filled" :disabled="lyricBusy">{{ lyricBusy ? "Mencari…" : "Cari" }}</button>
      </form>
      <button v-if="!lyrics.length" class="more" @click="loadLyrics(true)"><Icon name="sync" :size="18" />Cari ulang otomatis</button>
      <p v-if="lyricErr" class="ls-note">{{ lyricErr }}</p>
      <ul v-if="lyricHits" class="ls-hits">
        <li v-for="(h, i) in lyricHits" :key="i">
          <button @click="pickLyrics(h)">
            <span class="ls-title"><strong>{{ h.trackName }}</strong><span>{{ h.artistName }}{{ h.albumName ? ` · ${h.albumName}` : "" }}</span></span>
            <span v-if="h.syncedLyrics" class="ls-badge">Tersinkron</span>
            <span class="ls-dur">{{ h.duration ? fmt(h.duration) : "" }}</span>
          </button>
        </li>
        <li v-if="!lyricHits.length" class="ls-note">Tidak ada hasil untuk “{{ lyricQuery }}”.</li>
      </ul>
      <p v-else-if="!lyricErr" class="ls-note">Lirik dicari otomatis dari tag lagu, tapi tidak ketemu. Coba ubah kata kuncinya, atau letakkan file <code>.lrc</code> bernama sama di samping lagu.</p>
    </div>
    <div v-else class="lyrics" :class="{ synced: lyricIdx >= 0 || lyrics[0]?.t != null, paused: !playing }">
      <p v-for="(l, i) in lyrics" :key="i" :class="{ cur: i === lyricIdx, past: lyricIdx >= 0 && i < lyricIdx }"
        :style="lyricIdx >= 0 ? { '--d': Math.min(Math.abs(i - lyricIdx), 4) } : undefined">
        <span v-if="i === lyricIdx" :key="wipe.key" class="ln" :style="{ '--dur': `${wipe.dur}s`, '--delay': `${wipe.delay}s` }">{{ l.text || "♪" }}</span>
        <template v-else>{{ l.text || "♪" }}</template>
      </p>
      <div v-if="!lyrics.length" class="no-lyrics"><span>Memuat lirik…</span></div>
    </div>
  </section>
  </Transition>

  <Transition name="toast">
    <div v-if="toast" class="toast" :class="{ above: now }" role="alert">
      <Icon name="error" :size="20" />
      <span>{{ toast }}</span>
      <button class="icon-btn small" title="Tutup" @click="toast = ''"><Icon name="close" :size="18" /></button>
    </div>
  </Transition>
  <div v-if="downloads.length" class="downloads" :class="{ above: now }" role="status">
    <div v-for="d in downloads" :key="d.path" class="dl-item">
      <span class="dl-title">Mengunduh {{ d.title }}</span>
      <span class="dl-bar"><i :style="{ width: `${d.pct}%` }"></i></span>
      <span class="pct">{{ d.pct }}%</span>
    </div>
  </div>

  <footer v-if="now" class="player">
    <button class="np" @click="showNow = !showNow" :title="showNow ? 'Tutup layar putar' : 'Buka layar putar'">
      <CoverArt :path="now.path" />
      <span class="meta">
        <strong>{{ now.title }}</strong>
        <span>{{ unknown(now.artist) }}<template v-if="qualities[now.path]"> · {{ shortQuality(qualities[now.path]!) }}</template></span>
      </span>
    </button>
    <div class="ctrl">
      <div class="transport">
        <button class="icon-btn" :class="{ on: shuffle }" @click="shuffle = !shuffle" title="Acak" :aria-pressed="shuffle"><Icon name="shuffle" /></button>
        <button class="icon-btn tonal" @click="prev" title="Sebelumnya"><Icon name="prev" :size="28" /></button>
        <button class="play" :class="{ playing }" @click="togglePause" :title="paused ? 'Putar (Spasi)' : 'Jeda (Spasi)'">
          <Icon :name="paused ? 'play' : 'pause'" :size="32" />
        </button>
        <button class="icon-btn tonal" @click="next()" title="Berikutnya"><Icon name="next" :size="28" /></button>
        <button class="icon-btn" :class="{ on: repeat !== 'off' }" @click="cycleRepeat" :title="`Ulangi: ${repeatLabel}`" :aria-pressed="repeat !== 'off'">
          <Icon :name="repeat === 'one' ? 'repeatOne' : 'repeat'" />
        </button>
      </div>
      <div class="seek">
        <span>{{ fmt(position) }}</span>
        <WavyProgress :value="position" :max="now.duration_secs" :playing="playing" @scrub="scrub" @seek="seekTo" />
        <span>{{ fmt(now.duration_secs) }}</span>
      </div>
    </div>
    <div class="side">
      <button class="icon-btn" :class="{ on: showNow }" @click="showNow = !showNow" title="Lirik" :aria-pressed="showNow"><Icon name="lyrics" /></button>
      <Icon name="volume" />
      <input class="slider" type="range" min="0" max="1" step="0.01" v-model.number="volume"
        :style="{ '--p': `${volume * 100}%` }" aria-label="Volume"
        @input="invoke('set_volume', { volume })" />
    </div>
  </footer>
</template>

<style scoped>
.defs { position: absolute; width: 0; height: 0; }

/* ---------- layout ---------- */
.app {
  display: grid; grid-template-columns: var(--sidebar) 1fr;
  height: calc(100vh - var(--titlebar) - var(--sheet-gap));
  margin: var(--titlebar) var(--sheet-gap) 0; overflow: hidden;
  border-radius: var(--sheet-radius) var(--sheet-radius) 0 0; background: var(--surface);
}
.app.collapsed { grid-template-columns: 80px 1fr; }
.collapse-btn { margin-top: auto; align-self: center; }
.collapsed .lbl, .collapsed .badge { display: none; }
.collapsed .fab, .collapsed .dest-item { justify-content: center; padding: 0; }
.collapsed .fab { align-self: center; width: 56px; }
.collapsed .dest-item { width: 56px; align-self: center; border-radius: 16px; }
main { overflow-y: auto; padding: 0 40px 40px 16px; scrollbar-width: thin; scrollbar-color: var(--outline-variant) transparent; }
main.has-player { padding-bottom: 136px; }

/* ---------- navigation drawer ---------- */
.drawer { background: var(--surface-container-low); display: flex; flex-direction: column; gap: 16px; padding: 12px 12px 20px; overflow-y: auto; }
.drawer.has-player { padding-bottom: 148px; } /* keep the last item clear of the full-width player */
.fab {
  align-self: stretch; display: flex; align-items: center; gap: 12px;
  height: 56px; padding: 0 20px 0 16px; border-radius: 16px; font-size: 15px; font-weight: 600;
  background: var(--primary-container); color: var(--on-primary-container);
  box-shadow: 0 2px 6px rgb(0 0 0 / 0.25);
  transition: border-radius 0.35s var(--spring), transform 0.35s var(--spring);
}
.fab:hover, .fab.sel { border-radius: 28px; }
.fab:active { transform: scale(0.95); }
.dest { display: flex; flex-direction: column; gap: 2px; margin-top: 4px; }
.dest-item {
  display: flex; align-items: center; gap: 12px; height: 56px; padding: 0 16px; border-radius: 28px;
  font-size: 15px; font-weight: 600; color: var(--on-surface-variant); text-align: left;
  transition: background 0.25s var(--emph), color 0.25s var(--emph);
}
.dest-item:hover { background: var(--surface-container-high); color: var(--on-surface); }
.dest-item.sel { background: var(--secondary-container); color: var(--on-secondary-container); }
.badge { margin-left: auto; font-size: 13px; font-variant-numeric: tabular-nums; opacity: 0.8; }

/* ---------- header ---------- */
.top {
  position: sticky; top: 0; z-index: 4; background: var(--surface);
  display: flex; align-items: flex-end; justify-content: space-between; flex-wrap: wrap; gap: 16px 24px; padding: 12px 0 24px;
}
.heading { display: flex; align-items: center; gap: 16px; min-width: 0; }
h1 {
  margin: 0; font-size: clamp(40px, 5vw, 64px); line-height: 1; letter-spacing: -0.03em;
  font-weight: 800; font-variation-settings: "wdth" 118, "opsz" 72;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.top .actions { flex-basis: 100%; margin: 0; }
.sub { margin: 8px 0 0; color: var(--on-surface-variant); font-size: 15px; font-weight: 500; }
.search {
  display: flex; align-items: center; gap: 12px; height: 56px; padding: 0 8px 0 18px; width: min(360px, 40%);
  border-radius: 28px; background: var(--surface-container-high); color: var(--on-surface-variant);
  transition: background 0.2s;
}
.search:focus-within { background: var(--surface-container-highest); }
.search input { flex: 1; min-width: 0; border: 0; background: none; outline: none; font-size: 16px; color: var(--on-surface); }
.search input::placeholder { color: var(--on-surface-variant); }

/* ---------- buttons ---------- */
.btn-group { display: flex; flex-wrap: wrap; gap: 8px; }
.actions { margin-bottom: 20px; }
.btn {
  display: inline-flex; align-items: center; gap: 8px; height: 48px; padding: 0 24px 0 18px;
  border-radius: 24px; font-size: 15px; font-weight: 600;
  transition: border-radius 0.3s var(--spring), background 0.2s;
}
.btn.big { height: 64px; padding: 0 32px 0 24px; border-radius: 32px; font-size: 18px; font-variation-settings: "wdth" 110; }
.btn:active { border-radius: 12px; }
.btn:disabled { opacity: 0.5; cursor: default; }
.filled { background: var(--primary); color: var(--on-primary); }
.tonal { background: var(--secondary-container); color: var(--on-secondary-container); }
.outlined { box-shadow: inset 0 0 0 1px var(--outline-variant); color: var(--on-surface-variant); }
.outlined:hover { background: var(--surface-container-highest); }
.icon-btn {
  width: 40px; height: 40px; border-radius: 20px; display: inline-grid; place-items: center; flex: none;
  color: var(--on-surface-variant); transition: background 0.2s, border-radius 0.3s var(--spring);
}
.icon-btn:hover { background: var(--surface-container-highest); }
.icon-btn.tonal { color: var(--on-secondary-container); }
.icon-btn:active { border-radius: 10px; }
.icon-btn.on { color: var(--on-primary-container); background: var(--primary-container); }

/* ---------- grid ---------- */
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(176px, 1fr)); gap: 28px 20px; }
.tile { display: flex; flex-direction: column; gap: 4px; text-align: left; min-width: 0; }
.tile .art { width: 100%; height: auto; aspect-ratio: 1; margin-bottom: 10px; transition: transform 0.45s var(--spring); }
.tile:hover .art { transform: scale(1.05) rotate(-2deg); }
.tile strong { font-size: 16px; font-weight: 650; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.tile > span { font-size: 14px; color: var(--on-surface-variant); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.grid.artists .tile { align-items: center; text-align: center; }
.grid.artists .tile strong, .grid.artists .tile > span { max-width: 100%; }

/* ---------- catalog ---------- */
.catalog { display: flex; flex-direction: column; gap: 36px; }
.shelf-head { display: flex; align-items: baseline; justify-content: space-between; gap: 16px; margin-bottom: 16px; }
.shelf-head h2 { margin: 0; font-size: 24px; font-weight: 750; letter-spacing: -0.01em; font-variation-settings: "wdth" 112; }
.more { height: 36px; padding: 0 14px; border-radius: 18px; font-size: 14px; font-weight: 600; color: var(--primary); transition: background 0.2s; }
.more:hover { background: color-mix(in srgb, var(--primary) 8%, transparent); }
.shelf-row {
  display: grid; grid-auto-flow: column; grid-auto-columns: 168px; gap: 20px;
  overflow-x: auto; padding: 4px 4px 12px; margin: -4px -4px 0; scrollbar-width: thin; scrollbar-color: var(--outline-variant) transparent;
}
.shelf-row.artists .tile { align-items: center; text-align: center; }
.shelf-row.artists .tile strong, .shelf-row.artists .tile > span { max-width: 100%; }
.pl-art {
  width: 100%; aspect-ratio: 1; margin-bottom: 10px; display: grid; place-items: center;
  background: var(--secondary-container); color: var(--on-secondary-container); transition: transform 0.45s var(--spring);
}
.tile:hover .pl-art { transform: scale(1.05) rotate(-2deg); }

/* ---------- song list ---------- */
.songs { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 2px; }
.songs li { display: flex; align-items: center; gap: 8px; padding-right: 16px; border-radius: 16px; transition: background 0.2s; }
.songs li:hover { background: var(--surface-container); }
.songs li.active { background: var(--secondary-container); color: var(--on-secondary-container); }
.row { flex: 1; min-width: 0; display: flex; align-items: center; gap: 16px; padding: 8px 12px; text-align: left; border-radius: 16px; cursor: default; }
.art-wrap { position: relative; flex: none; }
.art-overlay { position: absolute; inset: 0; display: grid; place-items: center; border-radius: 12px; background: rgb(0 0 0 / 0.45); color: #fff; }
.meta { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
.meta strong { font-size: 16px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.downloads { position: fixed; right: 24px; bottom: 24px; z-index: 25; display: flex; flex-direction: column; gap: 8px; width: min(360px, calc(100vw - 48px)); }
.downloads.above { bottom: 128px; }
.toast {
  position: fixed; left: 50%; bottom: 24px; z-index: 26; transform: translateX(-50%); width: max-content; max-width: min(560px, calc(100vw - 32px));
  display: flex; align-items: center; gap: 12px; padding: 10px 8px 10px 16px; border-radius: 16px; font-size: 14px;
  background: var(--on-surface); color: var(--surface); box-shadow: 0 6px 20px rgb(0 0 0 / 0.3);
}
.toast.above { bottom: 128px; }
.toast .icon-btn { color: inherit; }
.toast-enter-active, .toast-leave-active { transition: opacity 0.2s, transform 0.3s var(--emph); }
.toast-enter-from, .toast-leave-to { opacity: 0; transform: translate(-50%, 12px); }
.dl-item { display: grid; grid-template-columns: 1fr auto; gap: 6px 12px; padding: 12px 16px; border-radius: 16px; background: var(--secondary-container); color: var(--on-secondary-container); box-shadow: 0 4px 16px rgb(0 0 0 / 0.25); font-size: 14px; }
.dl-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.dl-bar { grid-column: 1 / -1; grid-row: 2; height: 4px; border-radius: 2px; background: color-mix(in srgb, currentColor 20%, transparent); overflow: hidden; }
.dl-bar i { display: block; height: 100%; background: var(--primary); transition: width 0.2s; }
.dl-item .pct { font-variant-numeric: tabular-nums; font-weight: 600; }
.dl .pct { font-size: 12px; font-weight: 600; font-variant-numeric: tabular-nums; }
.cloud-mark { vertical-align: -2px; margin-right: 6px; color: var(--primary); }
.meta span { font-size: 14px; color: var(--on-surface-variant); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
li.active .meta span { color: inherit; opacity: 0.8; }
.dur { font-size: 14px; color: var(--on-surface-variant); font-variant-numeric: tabular-nums; min-width: 3.5ch; text-align: right; }
li.active .dur { color: inherit; }
.row-act { position: relative; opacity: 0; transition: opacity 0.15s; }
li:hover .row-act, .row-act:focus-visible, .row-menu.open .row-act, .row-act.open, .row-act.busy { opacity: 1; }
.row-menu { position: relative; }
.pl-menu {
  position: absolute; right: 0; top: calc(100% + 4px); z-index: 5; min-width: 200px; max-height: 280px; overflow-y: auto;
  display: flex; flex-direction: column; padding: 8px 0; border-radius: 16px;
  background: var(--surface-container-high); box-shadow: 0 4px 16px rgb(0 0 0 / 0.35);
}
.pl-menu p { margin: 0; padding: 4px 16px 8px; font-size: 12px; font-weight: 600; color: var(--on-surface-variant); }
.pl-menu button { padding: 10px 16px; text-align: left; font-size: 14px; }
.pl-menu button:hover, .pl-menu button:focus-visible { background: var(--surface-container-highest); }
.none { color: var(--on-surface-variant); font-size: 15px; padding: 24px 12px; }

.eq { display: inline-flex; align-items: flex-end; gap: 3px; height: 18px; }
.eq i { width: 4px; height: 100%; border-radius: 2px; background: currentColor; transform-origin: bottom; animation: eq 0.9s ease-in-out infinite alternate; }
.eq i:nth-child(2) { animation-delay: -0.3s; }
.eq i:nth-child(3) { animation-delay: -0.6s; }
.eq.still i { animation-play-state: paused; }
@keyframes eq { from { transform: scaleY(0.25); } to { transform: scaleY(1); } }

/* ---------- tabs (M3 primary, icon + label, scrollable) ---------- */
.tabs { flex-basis: 100%; display: flex; overflow-x: auto; scrollbar-width: none; box-shadow: inset 0 -1px var(--outline-variant); }
.tab {
  position: relative; display: flex; justify-content: center; min-width: 90px; height: 64px; padding: 0 16px;
  font-size: 14px; font-weight: 500; line-height: 20px; letter-spacing: 0.1px; color: var(--on-surface-variant);
  --state: var(--on-surface);
}
.tab::before { content: ""; position: absolute; inset: 0; background: var(--state); opacity: 0; transition: opacity 0.15s; }
.tab:hover::before { opacity: 0.08; }
.tab:focus-visible::before, .tab:active::before { opacity: 0.1; }
.tab-c { position: relative; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 2px; min-width: 24px; }
.tab.sel { color: var(--primary); --state: var(--primary); }
.tab.sel .tab-c::after {
  content: ""; position: absolute; left: 2px; right: 2px; bottom: 0; height: 3px;
  border-radius: 3px 3px 0 0; background: var(--primary);
}

/* ---------- chips ---------- */
.chips { display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 20px; }
.chip {
  display: inline-flex; align-items: center; height: 40px; border-radius: 12px;
  border: 1px solid var(--outline-variant); color: var(--on-surface-variant);
  transition: background 0.2s, border-radius 0.3s var(--spring);
}
.chip > button:first-child { height: 100%; padding: 0 16px; font-size: 14px; font-weight: 600; }
.chip:hover { background: var(--surface-container-high); }
.chip.sel { background: var(--secondary-container); color: var(--on-secondary-container); border-color: transparent; border-radius: 20px; }
.chip.sel > button:first-child { padding-right: 4px; }
.chip-x { width: 32px; height: 32px; margin-right: 4px; border-radius: 16px; display: grid; place-items: center; }
.chip.new { gap: 6px; padding: 0 6px 0 12px; border-style: dashed; }
.pl-note.verify { display: block; line-height: 1.6; }
.pl-note.verify input { display: block; width: 100%; margin-top: 6px; padding: 8px 12px; border-radius: 12px; border: 1px solid var(--outline-variant); background: var(--surface-container); }
.pl-note { display: flex; align-items: center; gap: 8px; margin: 0 0 12px; font-size: 14px; color: var(--on-surface-variant); }
.songs li.pending .row, .songs li.pending .dur { opacity: 0.75; }
.songs li.pending .row { cursor: default; }
.meta span.live { color: var(--primary); font-family: ui-monospace, monospace; font-size: 12px; }
.pend-art { width: 100%; height: 100%; display: grid; place-items: center; border-radius: 12px; background: var(--surface-container-high); color: var(--on-surface-variant); }
.chip.new input { width: 190px; border: 0; background: none; outline: none; font-size: 14px; }

/* ---------- empty state ---------- */
.empty { display: flex; flex-direction: column; align-items: center; text-align: center; padding: 8vh 0; }
.empty-mark {
  width: 200px; height: 200px; display: grid; place-items: center; margin-bottom: 28px;
  background: var(--primary-container); color: var(--on-primary-container);
  animation: spin 24s linear infinite;
}
.empty-mark .icon { animation: spin 24s linear infinite reverse; }
.empty h2 { margin: 0 0 8px; font-size: 32px; font-weight: 750; letter-spacing: -0.02em; font-variation-settings: "wdth" 115; }
.empty p { margin: 0 0 28px; color: var(--on-surface-variant); max-width: 42ch; font-size: 16px; line-height: 1.5; }

.scanning { display: flex; align-items: center; gap: 12px; color: var(--on-surface-variant); margin: 0 0 20px 12px; }
.spinner { width: 24px; height: 24px; background: var(--primary); animation: spin 1.2s linear infinite, pulse 1.2s var(--emph) infinite alternate; }
@keyframes spin { to { transform: rotate(360deg); } }
@keyframes pulse { from { scale: 0.8; } to { scale: 1.05; } }

/* ---------- S3 panel ---------- */
.cloud { display: flex; flex-direction: column; gap: 16px; }
.panel.conn { max-width: none; }
.conn-head { display: flex; align-items: center; gap: 16px; flex-wrap: wrap; }
.conn-mark { width: 56px; height: 56px; display: grid; place-items: center; flex: none; background: var(--primary-container); color: var(--on-primary-container); }
.conn-id { flex: 1; min-width: 200px; display: flex; flex-direction: column; gap: 2px; }
.conn-id strong { font-size: 20px; font-weight: 700; }
.conn-id span { color: var(--on-surface-variant); font-size: 14px; overflow-wrap: anywhere; }
.conn-edit { width: 48px; height: 48px; border-radius: 24px; }
.folder-chips { margin: 0; }
.folder-chips .chip > button:first-child { display: inline-flex; align-items: center; gap: 8px; }
.conn-form { display: flex; flex-direction: column; gap: 20px; max-width: 640px; }
.panel { max-width: 640px; padding: 28px; border-radius: 28px; background: var(--surface-container-low); display: flex; flex-direction: column; gap: 20px; }
.hint { margin: 0; color: var(--on-surface-variant); line-height: 1.5; }
.fields { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
.field { display: flex; flex-direction: column; gap: 6px; font-size: 13px; font-weight: 600; color: var(--on-surface-variant); }
.field.wide { grid-column: 1 / -1; }
.field input {
  height: 52px; padding: 0 16px; border: 0; border-radius: 12px 12px 4px 4px; outline: none;
  background: var(--surface-container-highest); color: var(--on-surface); font-size: 15px; font-weight: 400;
  box-shadow: inset 0 -1px 0 var(--on-surface-variant); transition: box-shadow 0.2s;
}
.field input:focus { box-shadow: inset 0 -3px 0 var(--primary); }
.field input::placeholder { color: var(--on-surface-variant); opacity: 0.7; }
.status { margin: 0; padding: 12px 16px; border-radius: 12px; background: var(--secondary-container); color: var(--on-secondary-container); }
.status { display: flex; align-items: center; gap: 8px; }
.status > span { flex: 1; }
.status .icon-btn { width: 32px; height: 32px; color: inherit; margin: -6px -8px -6px 0; }
.fade-enter-active, .fade-leave-active { transition: opacity 0.25s var(--emph); }
.fade-enter-from, .fade-leave-to { opacity: 0; }
.status.err { background: #8c1d18; color: #ffdad6; }

/* ---------- player bar ---------- */
.player {
  position: fixed; left: 16px; right: 16px; bottom: 16px; z-index: 20;
  /* equal outer columns keep the controls centred on the window at every width */
  display: grid; grid-template-columns: minmax(0, 1fr) minmax(260px, 1.6fr) minmax(0, 1fr);
  align-items: center; gap: 20px; padding: 6px 16px 6px 6px;
  border-radius: 28px; background: var(--surface-container-high);
  /* the surface-coloured spread masks list rows scrolling past the floating bar */
  box-shadow: 0 6px 24px rgb(0 0 0 / 0.28), 0 0 0 calc(10px - var(--sheet-gap)) var(--surface);
}
.np { display: flex; align-items: center; gap: 14px; min-width: 0; text-align: left; border-radius: 16px; padding-right: 8px; }
.np .art { width: 64px; height: 64px; border-radius: 16px; transition: border-radius 0.4s var(--spring); }
.np:hover .art { border-radius: 32px; }
.ctrl { display: flex; flex-direction: column; align-items: stretch; gap: 4px; width: 100%; max-width: 560px; justify-self: center; }
.transport { display: flex; align-items: center; justify-content: center; gap: 8px; }
@media (max-width: 1100px) {
  .player { gap: 12px; }
  .transport { gap: 4px; }
  .seek { gap: 8px; }
}
.transport .icon-btn.tonal { width: 48px; height: 48px; border-radius: 24px; background: var(--secondary-container); }
.transport .icon-btn.tonal:active { border-radius: 12px; }
.play {
  width: 76px; height: 56px; border-radius: 28px; display: grid; place-items: center;
  background: var(--primary); color: var(--on-primary);
  transition: border-radius 0.45s var(--spring), transform 0.3s var(--spring);
}
.play.playing { border-radius: 18px; }
.play:active { transform: scale(0.94); }
.seek { display: flex; align-items: center; gap: 12px; font-size: 12px; font-weight: 600; color: var(--on-surface-variant); font-variant-numeric: tabular-nums; }
.side { display: flex; align-items: center; justify-content: flex-end; gap: 8px; min-width: 0; color: var(--on-surface-variant); }
.slider {
  appearance: none; -webkit-appearance: none; width: clamp(56px, 8vw, 128px); height: 16px; border-radius: 8px; margin: 0; cursor: pointer;
  background: linear-gradient(to right, var(--primary) var(--p), var(--secondary-container) var(--p));
}
.slider::-webkit-slider-thumb {
  -webkit-appearance: none; width: 4px; height: 32px; border-radius: 2px; background: var(--primary);
  box-shadow: 0 0 0 4px var(--surface-container-high);
}
.slider::-moz-range-thumb { width: 4px; height: 32px; border: 0; border-radius: 2px; background: var(--primary); }

/* ---------- now playing ---------- */
.now {
  position: fixed; inset: var(--titlebar) var(--sheet-gap) var(--sheet-gap); z-index: 10;
  display: grid; grid-template-columns: 1fr 1.1fr; gap: 48px;
  padding: 40px 64px 132px; background: var(--surface); border-radius: var(--sheet-radius) var(--sheet-radius) 0 0;
}
/* opens rising from the player, closes sinking back into it */
.now-enter-active { transition: opacity 0.35s var(--emph), transform 0.5s var(--emph); }
.now-leave-active { transition: opacity 0.25s var(--emph), transform 0.3s cubic-bezier(0.3, 0, 0.8, 0.15); }
.now-enter-from { opacity: 0; transform: translateY(32px); }
.now-leave-to { opacity: 0; transform: translateY(48px) scale(0.98); }
.now-leave-active .spin { transition: transform 0.3s var(--emph); transform: scale(0.9); }
.close { position: absolute; top: 12px; left: 12px; width: 48px; height: 48px; border-radius: 24px; background: var(--secondary-container); }
.stage { display: flex; flex-direction: column; justify-content: center; align-items: center; text-align: center; min-width: 0; }
.spin {
  width: min(52vh, 34vw); aspect-ratio: 1; display: grid; place-items: center; overflow: hidden;
  background: var(--primary-container); color: var(--on-primary-container);
}
.spin img { width: 100%; height: 100%; object-fit: cover; }
.stage h2 {
  margin: 32px 0 8px; font-size: clamp(28px, 3.4vw, 48px); line-height: 1.05; letter-spacing: -0.03em;
  font-weight: 800; font-variation-settings: "wdth" 120, "opsz" 60; text-wrap: balance;
}
.stage p { margin: 0; color: var(--primary); font-size: 18px; font-weight: 600; }
.stage p.now-q {
  margin-top: 10px; display: inline-block; padding: 4px 12px; border-radius: 12px; font-size: 13px; font-weight: 700;
  font-variant-numeric: tabular-nums; background: var(--tertiary-container); color: var(--on-tertiary-container);
}
.lyrics {
  overflow-y: auto; padding: 30vh 8px; scrollbar-width: none;
  mask-image: linear-gradient(transparent, #000 22%, #000 78%, transparent);
}
.lyrics p {
  margin: 0 0 18px; font-size: clamp(22px, 2.2vw, 32px); line-height: 1.25; font-weight: 750;
  letter-spacing: -0.02em; color: var(--on-surface); opacity: 0.35;
  transform-origin: left center; transition: opacity 0.35s, transform 0.5s var(--spring), color 0.35s;
}
.lyrics:not(.synced) p { opacity: 0.85; font-size: 20px; font-weight: 500; }
/* lines fade and soften with distance from the current one */
.lyrics.synced p {
  opacity: max(0.16, calc(0.62 - var(--d, 2) * 0.12)); filter: blur(calc(max(0, var(--d, 0) - 1) * 0.6px));
  transition: opacity 0.5s var(--emph), transform 0.6s var(--spring), color 0.4s, filter 0.5s var(--emph), font-size 0.5s var(--emph);
}
.lyrics.synced p.past { opacity: max(0.12, calc(0.45 - var(--d, 2) * 0.1)); }
.lyrics p.cur {
  opacity: 1; filter: none;
  font-size: clamp(28px, 3vw, 44px); transform: translateX(4px); animation: lift 0.6s var(--spring);
}
@keyframes lift { from { transform: scale(0.98) translateY(10px); } }
/* the current line fills with colour as it is sung */
.ln {
  background: linear-gradient(90deg, var(--primary) 50%, color-mix(in srgb, var(--primary) 32%, var(--on-surface) 20%) 50%) 100% 0 / 200% 100%;
  -webkit-background-clip: text; background-clip: text; color: transparent;
  animation: wipe var(--dur) linear var(--delay) forwards;
}
.lyrics.paused .ln { animation-play-state: paused; }
@keyframes wipe { to { background-position: 0 0; } }
@media (prefers-reduced-motion: reduce) {
  .lyrics p.cur { animation: none; }
  .ln { animation: none; background: none; color: var(--primary); }
}
.no-lyrics { height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; color: var(--on-surface-variant); text-align: center; }
.lyrics .no-lyrics p { opacity: 1; font-size: 22px; color: var(--on-surface); margin: 8px 0 0; }
.lyric-actions { position: absolute; top: 12px; right: 12px; z-index: 2; }
.lyric-actions > .icon-btn { width: 48px; height: 48px; border-radius: 24px; background: var(--secondary-container); }
.lyric-actions .pl-menu button { display: flex; align-items: center; gap: 12px; padding: 12px 16px; color: var(--on-surface); }
.lyric-actions .pl-menu .icon { color: var(--on-surface-variant); }
.menu-enter-active { transition: opacity 0.15s, transform 0.25s var(--spring); transform-origin: top right; }
.menu-leave-active { transition: opacity 0.1s; }
.menu-enter-from { opacity: 0; transform: scale(0.9); }
.menu-leave-to { opacity: 0; }
.lyric-search { display: flex; flex-direction: column; gap: 16px; min-height: 0; padding-top: 24px; }
.ls-head { display: flex; align-items: flex-start; gap: 16px; color: var(--on-surface-variant); }
.ls-head > div { flex: 1; display: flex; flex-direction: column; gap: 4px; }
.ls-head strong { font-size: 24px; font-weight: 750; color: var(--on-surface); }
.ls-bar { display: flex; gap: 8px; }
.ls-bar .search { flex: 1; width: auto; }
.lyric-search > .more { align-self: flex-start; display: inline-flex; align-items: center; gap: 6px; }
.ls-note { margin: 0; color: var(--on-surface-variant); line-height: 1.5; }
.ls-note code { background: var(--surface-container-high); padding: 2px 6px; border-radius: 6px; }
.ls-hits { list-style: none; margin: 0; padding: 0; overflow-y: auto; min-height: 0; display: flex; flex-direction: column; gap: 2px; scrollbar-width: thin; }
.ls-hits button {
  width: 100%; display: flex; align-items: center; gap: 12px; padding: 12px 16px; border-radius: 16px; text-align: left;
  transition: background 0.2s;
}
.ls-hits button:hover { background: var(--surface-container-high); }
.ls-title { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.ls-title strong, .ls-title span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ls-title strong { font-size: 16px; font-weight: 600; }
.ls-title span { font-size: 14px; color: var(--on-surface-variant); }
.ls-badge { flex: none; padding: 4px 10px; border-radius: 8px; font-size: 12px; font-weight: 600; background: var(--tertiary-container); color: var(--on-tertiary-container); }
.ls-dur { flex: none; width: 44px; text-align: right; font-size: 14px; color: var(--on-surface-variant); font-variant-numeric: tabular-nums; }
.no-lyrics code { background: var(--surface-container-high); padding: 2px 6px; border-radius: 6px; }

/* player: below the desktop minimum (960px) the seek bar stacks under the controls */
@media (max-width: 1100px) {
  /* small windows: lyrics take the screen; the artwork goes, title and lyric actions stay on top */
  .now { grid-template-columns: 1fr; grid-template-rows: auto minmax(0, 1fr); gap: 8px; padding: 64px 32px 132px; }
  .spin { display: none; }
  .stage h2 { margin: 0 0 4px; font-size: clamp(24px, 3vw, 32px); }
  .lyrics { padding: 12vh 8px; text-align: center; }
  .lyrics p { transform-origin: center; }
  .lyric-search { padding-top: 8px; }
}
@media (max-width: 860px) {
  .player {
    grid-template-columns: minmax(0, 1fr) auto auto; grid-template-areas: "np transport side" "seek seek seek";
    gap: 0 8px; padding: 6px 12px 4px 6px; border-radius: 24px;
  }
  .ctrl { display: contents; }
  .np { grid-area: np; }
  .transport { grid-area: transport; }
  .side { grid-area: side; }
  .seek { grid-area: seek; padding: 0 4px 0 8px; min-width: 0; }
  .np .art { width: 52px; height: 52px; border-radius: 14px; }
}
@media (max-width: 600px) {
  .top .search { width: 100%; }
  main { padding: 0 16px 40px 8px; }
  .player { left: 8px; right: 8px; bottom: 8px; padding-right: 8px; }
  .transport { gap: 4px; }
  .transport > .icon-btn:not(.tonal), .side { display: none; }
  .transport .icon-btn.tonal { width: 40px; height: 40px; }
  .play { width: 52px; height: 44px; }
  .np .art { width: 44px; height: 44px; }
  .np { gap: 10px; }
  .meta strong { font-size: 14px; }
  .meta span { font-size: 12px; }
  .seek > span { display: none; }
}
@media (prefers-reduced-motion: reduce) {
  .empty-mark, .empty-mark .icon, .eq i { animation: none; }
}
</style>
