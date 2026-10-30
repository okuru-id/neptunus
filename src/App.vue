<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { documentDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { computed, onMounted, onUnmounted, reactive, ref, watch, watchEffect } from "vue";
import CoverArt from "./lib/CoverArt.vue";
import { covers } from "./lib/covers";
import Icon from "./lib/Icon.vue";
import ConfirmDialog, { confirm, prompt } from "./lib/ConfirmDialog.vue";
import { buildMix, find as findIn, kind as mixKind, savePlaylist } from "./lib/mix";
import Dropdown from "./lib/Dropdown.vue";
import FolderPicker, { chooseFolder } from "./lib/FolderPicker.vue";
import FileManager from "./lib/FileManager.vue";
import Extensions from "./lib/Extensions.vue";
import Equalizer from "./lib/Equalizer.vue";
import About from "./lib/About.vue";
import Onboarding, { firstRun } from "./lib/Onboarding.vue";
import ExtCatalog from "./lib/ExtCatalog.vue";
import VideoLibrary from "./lib/VideoLibrary.vue";
import { installUpdate, updater } from "./lib/updater";
import SongMenu from "./lib/SongMenu.vue";
import { downloadError, extName, has, streamExt, streamQuality, unsupported, type ExtInfo, type More } from "./lib/ext";
import { asked, needsVerify, verified, withVerify } from "./lib/verify";
import { refreshCached } from "./lib/vcache";
import { mpv, setEngine, type Engine } from "./lib/mpv";
import { activity, catalogDl, mixing } from "./lib/activity";
import { loadQuality, longQuality, qualities, shortQuality } from "./lib/quality";
import TitleBar from "./lib/TitleBar.vue";
import Notifications, { type Job, type Note } from "./lib/Notifications.vue";
import type { IconName } from "./lib/icons";
import WavyProgress from "./lib/WavyProgress.vue";
import { applyTheme, DEFAULT_SEED, seedFromImage, setThemeMode, themeMode, type ThemeMode } from "./lib/theme";
import { ART_SHAPES, CLIP_SHAPES } from "./lib/shapes";

type Track = { path: string; title: string; artist: string; album: string; duration_secs: number };
type Playlist = { id: number; name: string };
type View = "catalog" | "songs" | "albums" | "artists" | "playlists" | "videos" | "s3";

const NAV: { id: View; label: string; icon: IconName }[] = [
  { id: "catalog", label: "Katalog", icon: "catalog" },
  { id: "songs", label: "Lagu", icon: "note" },
  { id: "albums", label: "Album", icon: "album" },
  { id: "artists", label: "Artis", icon: "artist" },
  { id: "playlists", label: "Playlist", icon: "playlist" },
  { id: "videos", label: "Video", icon: "movie" },
];

const rawLibrary = ref<Track[]>([]);
// A cloud song is hidden when a local one has the same title and artist (and about the same length).
const sameSong = (t: Track) => `${t.title.trim().toLowerCase()}\u0000${t.artist.trim().toLowerCase()}`;
// While S3 is disconnected its songs stay in the database (so reconnecting needs no re-read) but are hidden.
const s3Visible = (t: Track) => s3Saved.value || !t.path.startsWith("s3://");
const visibleLibrary = computed(() => rawLibrary.value.filter(s3Visible));
const library = computed(() => {
  const local = new Map<string, number[]>();
  for (const t of visibleLibrary.value) if (!t.path.startsWith("s3://")) local.set(sameSong(t), [...(local.get(sameSong(t)) ?? []), t.duration_secs]);
  return visibleLibrary.value.filter(
    (t) => !t.path.startsWith("s3://") || !local.get(sameSong(t))?.some((d) => Math.abs(d - t.duration_secs) <= 3),
  );
});
const playlists = ref<Playlist[]>([]);
const loading = ref(false);

// the page that was open survives a reload (not a restart: sessionStorage)
const PAGE_KEY = "neptunus.page";
const page = (() => { try { return JSON.parse(sessionStorage.getItem(PAGE_KEY) ?? "{}"); } catch { return {}; } })();
const view = ref<View>(page.view ?? "catalog");
const query = ref("");
const videoCount = ref(0);
const watching = ref(false); // a film is open: settings stays hidden so the player is not unmounted
const videoDetail = ref(false); // a film's detail page is open, so the search hides
const group = ref<string | null>(page.group ?? null); // selected album/artist name
const plId = ref<number | null>(page.plId ?? null);
const plTracks = ref<Track[]>([]);
const newPlaylist = ref("");

type S3Config = { endpoint: string; region: string; bucket: string; prefix: string; access_key: string; secret_key: string; disconnected?: boolean };
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
const buffered = ref<number | null>(null); // fraction of an S3 stream fetched ahead, for the progress bar
const volume = ref(0.8);
const shuffle = ref(false);
const sidebarOpen = ref(localStorage.getItem("sidebar") !== "0");
// narrow windows always get the mini rail
const narrowMq = matchMedia("(max-width: 900px)");
const narrow = ref(narrowMq.matches);
narrowMq.addEventListener("change", (e) => (narrow.value = e.matches));
// phones: a song row's buttons fold into its ⋮ menu
const phoneMq = matchMedia("(max-width: 600px)");
const phone = ref(phoneMq.matches);
phoneMq.addEventListener("change", (e) => (phone.value = e.matches));
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
  if (view.value === "playlists") return plTracks.value.filter((t) => s3Visible(t) && match(t));
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
  if (view.value === "videos") return `${videoCount.value} video`;
  if (view.value === "catalog") return `${library.value.length} lagu · ${albums.value.length} album · ${artists.value.length} artis`;
  if (view.value === "albums" && group.value === null) return `${albums.value.length} album`;
  if (view.value === "artists" && group.value === null) return `${artists.value.length} artis`;
  // a playlist counts its songs not yet downloaded too
  const songs = [...shown.value, ...(view.value === "playlists" ? plPending.value : [])];
  const total = songs.reduce((s, t) => s + t.duration_secs, 0);
  return `${songs.length} lagu · ${Math.round(total / 60)} menit`;
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
type Pending = { id: number; title: string; artist: string; album: string; duration_secs: number; uri: string; cover: string };
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
// a mix being made opens the bell (its progress shows there); done, the bell closes and says so
const bell = ref<{ show: () => void; close: () => void }>();
watch(() => Object.keys(mixing).length, (n, old) => (n && !old ? bell.value?.show() : !n && old && bell.value?.close()));
// Playlist › Buat mix: from a song or artist typed in (the playing song to start with), searched in the
// catalog picked last, saved straight away as a playlist
async function mixPlaylist() {
  const metas = exts.value.filter((e) => e.enabled && has(e, "metadata_provider"));
  const ext = metas.find((e) => e.id === localStorage.getItem("neptunus.extSource")) ?? metas[0];
  if (!ext) return notify("Pasang ekstensi katalog (misalnya Spotify Web) untuk membuat mix.");
  const q = await prompt({
    title: "Buat mix", icon: "playlist", confirm: "Buat mix", placeholder: "Judul lagu atau artis",
    message: `Ketik lagu atau artis. Playlist sampai 30 lagu dibuat dari lagu teratas hasil pencariannya di ${extName(ext)}.`,
    input: now.value ? `${now.value.title} ${now.value.artist}` : "",
  });
  if (!q) return;
  const job = `pl:${q}`;
  mixing[job] = q;
  try {
    const seed = (await findIn(ext.id, q)).find((i) => mixKind(i) === "track");
    if (!seed) throw `tidak ada lagu yang cocok dengan “${q}”`;
    const { tracks } = await buildMix(ext.id, seed);
    const r = await savePlaylist(`Mix · ${seed.name}`, tracks);
    await refreshPlaylists();
    await openPlaylist(r.id);
    notify(`Mix “${seed.name}” selesai: ${tracks.length} lagu`, "done");
  } catch (e) {
    notify(`Mix gagal: ${e}`);
  } finally {
    delete mixing[job];
  }
}
function mixed(title: string, res: number | string) {
  if (typeof res === "number") notify(`Mix “${title}” selesai: ${res} lagu`, "done");
  else notify(`Mix “${title}” gagal: ${res}`);
}
async function mixSaved(id: number, name: string) {
  await refreshPlaylists();
  notify(`Playlist “${name}” disimpan`, "done", { label: "Buka", run: () => { setView("playlists"); openPlaylist(id); } });
}
async function importSpotify(url: string) {
  plNote.value = "Membaca playlist Spotify…";
  try {
    const r = await invoke<{ id: number; name: string; added: number; pending: number }>("spotify_import", { url });
    plNote.value = `“${r.name}”: ${r.added} lagu dari library, ${r.pending} belum diunduh (unduh per lagu atau dengan Unduh semua).`;
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
    const title = rawLibrary.value.find((t) => t.path === path)?.title ?? path;
    rawLibrary.value = await invoke<Track[]>("s3_download", { path });
    addNote("done", `“${title}” tersimpan di folder Musik.`);
  } catch (e) {
    s3Status.value = `Gagal mengunduh: ${e}`;
    addNote("error", `Gagal mengunduh dari S3: ${e}`);
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

// the sticky header's height, so bars inside a view (the video tabs) can stick right under it
const topEl = ref<HTMLElement>();
const topH = ref(0);
const topObs = new ResizeObserver(([e]) => (topH.value = (e.target as HTMLElement).offsetHeight));
watch(topEl, (el, old) => { if (old) topObs.unobserve(old); if (el) topObs.observe(el); });
const videoHome = ref(0); // the Video menu clicked while already there: back to the catalog
function setView(v: View) {
  if (v === "videos" && view.value === "videos") videoHome.value++;
  view.value = v;
  group.value = null;
  // songs downloaded elsewhere (catalog) since take their place in the open playlist
  if (v === "playlists" && plId.value !== null) openPlaylist(plId.value);
}

type SettingsTab = "local" | "cache" | "s3" | "eq" | "look" | "ext" | "about";
const SETTINGS_TABS: { id: SettingsTab; label: string; icon: IconName }[] = [
  { id: "about", label: "Tentang", icon: "info" },
  { id: "local", label: "Lokal", icon: "folder" },
  { id: "cache", label: "Cache", icon: "storage" },
  { id: "s3", label: "Cloud", icon: "cloud" },
  { id: "eq", label: "Equalizer", icon: "equalizer" },
  { id: "look", label: "Tampilan", icon: "palette" },
  { id: "ext", label: "Ekstensi", icon: "extension" },
];
const THEME_MODES: { id: ThemeMode; label: string; icon: IconName }[] = [
  { id: "system", label: "Sistem", icon: "autoMode" },
  { id: "light", label: "Terang", icon: "lightMode" },
  { id: "dark", label: "Gelap", icon: "darkMode" },
];
const settingsTab = ref<SettingsTab>(SETTINGS_TABS.some((t) => t.id === page.settingsTab) ? page.settingsTab! : "about");
watch([view, group, plId, settingsTab], ([view, group, plId, settingsTab]) => {
  try { sessionStorage.setItem(PAGE_KEY, JSON.stringify({ view, group, plId, settingsTab })); } catch {}
});
// Settings › Tampilan › Pemutar film (with an outside player here: mpv, a player app, VLC or Infuse)
const ENGINES: { id: Engine; label: string; icon: IconName }[] = [
  { id: "ask", label: "Tanya", icon: "info" },
  { id: "builtin", label: "Aplikasi", icon: "movie" },
  { id: "mpv", label: "", icon: "play" }, // `mpv.label`
];

// Settings › Cache: the disk cache online films play from (Rust `video_cache_*`, `serve::prune`)
const CACHE_DAYS = [1, 3, 7, 14, 30].map((d) => ({ value: d, label: `${d} hari` }));
const cacheSize = ref<number | null>(null);
const cacheDays = ref(7);
const bytes = (n: number) => (n >= 2 ** 30 ? `${(n / 2 ** 30).toFixed(1)} GB` : `${Math.round(n / 2 ** 20)} MB`);
// the films in it, to delete some by hand
type CachedFilm = { key: string; title: string | null; size: number; done: number; used: number };
const cacheFilms = ref<CachedFilm[]>([]);
const cachePick = ref(new Set<string>());
const pickedSize = computed(() => cacheFilms.value.filter((f) => cachePick.value.has(f.key)).reduce((n, f) => n + f.size, 0));
function togglePick(key: string) {
  const s = new Set(cachePick.value);
  if (!s.delete(key)) s.add(key);
  cachePick.value = s;
}
const pickAll = () => (cachePick.value = cachePick.value.size === cacheFilms.value.length ? new Set() : new Set(cacheFilms.value.map((f) => f.key)));
const playedOn = (secs: number) => (secs ? new Date(secs * 1000).toLocaleDateString("id-ID", { day: "numeric", month: "short" }) : "");
async function removePicked() {
  const n = cachePick.value.size;
  const ok = await confirm({
    title: `Hapus ${n} film dari cache?`,
    message: `${bytes(pickedSize.value)} dikosongkan. Film itu dimuat dari awal lagi kalau diputar nanti.`,
    confirm: "Hapus", icon: "trash", danger: true,
  });
  if (!ok) return;
  await invoke("video_cache_remove", { keys: [...cachePick.value] }).catch((e) => notify(String(e)));
  cachePick.value = new Set();
  await loadCache();
  refreshCached(); // the cache marks in the catalogs
}
async function loadCache() {
  cacheFilms.value = await invoke<CachedFilm[]>("video_cache_list").catch(() => []);
  cachePick.value = new Set([...cachePick.value].filter((k) => cacheFilms.value.some((f) => f.key === k)));
  cacheSize.value = await invoke<number>("video_cache_size").catch(() => 0);
  cacheDays.value = Number(await invoke<string | null>("setting_get", { key: "video_cache_days" }).catch(() => null)) || 7;
}
function setCacheDays(d: number) {
  cacheDays.value = d;
  invoke("setting_set", { key: "video_cache_days", value: String(d) }).catch((e) => notify(String(e)));
}
async function clearCache() {
  const ok = await confirm({
    title: "Hapus cache film?",
    message: "Buffer film online dan poster katalog yang tersimpan dihapus. Film yang diputar lagi akan dimuat dari awal, dan poster diunduh lagi saat katalog dibuka.",
    confirm: "Hapus", icon: "trash", danger: true,
  });
  if (!ok) return;
  await invoke("video_cache_clear").catch((e) => notify(String(e)));
  await loadCache();
  refreshCached();
}
// Settings › Cache: songs played by streaming (Rust `ext_stream`), kept until the 1 GB limit pushes them out
type CachedSong = { path: string; title: string; artist: string; size: number; used: number };
const cacheSongs = ref<CachedSong[]>([]);
// the cache cards start folded: only their size shows until opened
const cacheOpen = reactive({ film: false, music: false });
const songPick = ref(new Set<string>());
const songsSize = computed(() => cacheSongs.value.reduce((n, f) => n + f.size, 0));
const pickedSongsSize = computed(() => cacheSongs.value.filter((f) => songPick.value.has(f.path)).reduce((n, f) => n + f.size, 0));
function toggleSong(path: string) {
  const s = new Set(songPick.value);
  if (!s.delete(path)) s.add(path);
  songPick.value = s;
}
const pickAllSongs = () => (songPick.value = songPick.value.size === cacheSongs.value.length ? new Set() : new Set(cacheSongs.value.map((f) => f.path)));
async function loadSongs() {
  cacheSongs.value = await invoke<CachedSong[]>("stream_cache_list").catch(() => []);
  songPick.value = new Set([...songPick.value].filter((k) => cacheSongs.value.some((f) => f.path === k)));
}
async function removeSongs(all: boolean) {
  const n = all ? cacheSongs.value.length : songPick.value.size;
  const ok = await confirm({
    title: all ? "Hapus cache musik?" : `Hapus ${n} lagu dari cache?`,
    message: `${bytes(all ? songsSize.value : pickedSongsSize.value)} dikosongkan. Lagu itu diambil lagi kalau diputar nanti.`,
    confirm: "Hapus", icon: "trash", danger: true,
  });
  if (!ok) return;
  await (all ? invoke("stream_cache_clear") : invoke("stream_cache_remove", { paths: [...songPick.value] })).catch((e) => notify(String(e)));
  await loadSongs();
}
watch(() => view.value === "s3" && settingsTab.value === "cache", (on) => on && (loadCache(), loadSongs()), { immediate: true });

function openSettings(tab: SettingsTab) {
  settingsTab.value = tab;
  setView("s3");
  showNow.value = false; // the now-playing/lyrics screen covers the page
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
    addNote("done", `“${t.title}” selesai diunduh ulang.`);
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
const pdlTitle: Record<number, string> = {};
// notices kept in the title bar's bell (newest first), and how many arrived since it was last opened
const notes = ref<Note[]>([]);
const unread = ref(0);
let noteId = 0;
function addNote(kind: Note["kind"], text: string, url?: string) {
  notes.value = [{ id: ++noteId, kind, text, time: Date.now(), url }, ...notes.value].slice(0, 50);
  unread.value++;
}
// short-lived notice at the bottom of the window (a failed download, ...), also kept in the bell
const toast = ref("");
const toastAction = ref<{ label: string; run: () => void } | null>(null);
let toastTimer: ReturnType<typeof setTimeout> | undefined;
function notify(text: string, kind: Note["kind"] = "error", action: { label: string; run: () => void } | null = null) {
  addNote(kind, text);
  toast.value = text;
  toastAction.value = action;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toast.value = ""), 7000);
}
// a newer GitHub release, checked once per launch; closing it skips that version
const update = ref<{ version: string; url: string } | null>(null);
async function checkUpdate() {
  const r = await invoke<[string, string] | null>("check_update").catch(() => null);
  if (r) addNote("update", `Neptunus versi ${r[0]} tersedia.`, r[1]);
  if (r && localStorage.getItem("skipUpdate") !== r[0]) update.value = { version: r[0], url: r[1] };
}
function skipUpdate() {
  localStorage.setItem("skipUpdate", update.value!.version);
  update.value = null;
}
function onDownloadFailed(song: string, ext: string, err: unknown) {
  if (needsVerify(err)) return; // the snackbar from `asked` below says it once for all of them
  const e = exts.value.find((x) => x.id === ext);
  notify(downloadError(err, song, e ? extName(e) : ext));
}
// a download needed a verification (session expired or never done): point to Settings › Verifikasi,
// once per extension for a while, since a whole playlist may run into it at once
const askedAt: Record<string, number> = {};
watch(asked, (a) => {
  if (!a || Date.now() - (askedAt[a.id] ?? 0) < 30_000) return;
  askedAt[a.id] = Date.now();
  const e = exts.value.find((x) => x.id === a.id);
  notify(`Sesi ${e ? extName(e) : a.id} habis atau belum diverifikasi.`, "error", { label: "Verifikasi", run: () => openSettings("ext") });
});
// pending playlist song -> why its last download failed
const pfail = ref<Record<number, string>>({});
// a pending playlist song as the catalog knows it (full metadata, cover, album), so it downloads the same way
async function catalogTrack(p: Pending) {
  const meta = exts.value.find((e) => e.enabled && e.id === "spotify-web");
  const sid = p.uri.split(":")[2];
  return meta && sid
    ? await invoke<Record<string, unknown> | null>("ext_call", { id: meta.id, method: "getTrack", args: [sid] })
        .then((t) => (t ? { ...t, provider_id: meta.id } : null)).catch(() => null)
    : null;
}
// for streaming, where nothing else fills it in: what the playlist knows, looked up by the extension
const pendingTrack = async (p: Pending) => (await catalogTrack(p)) ??
  { id: "", name: p.title, artists: p.artist, album_name: p.album, duration_ms: p.duration_secs * 1000, cover_url: p.cover, provider_id: "playlist" };
async function downloadPending(p: Pending, ext: string, quality: string | null) {
  const { [p.id]: _, ...ok } = pfail.value;
  pfail.value = ok;
  pdlExt[p.id] = ext;
  pdlTitle[p.id] = p.title;
  pdl.value = { ...pdl.value, [p.id]: 0 };
  activity[ext] = "";
  try {
    const track = await catalogTrack(p);
    await withVerify(ext, () => invoke("pending_download", { id: p.id, ext, quality, track }));
    addNote("done", `“${p.title}” selesai diunduh.`);
    setLibrary(await invoke<Track[]>("load_library"));
    if (plId.value !== null) await openPlaylist(plId.value);
  } catch (e) {
    if (String(e).includes("dibatalkan")) return; // cancelled by the user: no error to show
    // like the catalog: no dialog, the row's button turns into "try again" with the reason as its tooltip
    pfail.value = { ...pfail.value, [p.id]: String(e) };
    onDownloadFailed(p.title, ext, e);
  } finally {
    const { [p.id]: _, ...rest } = pdl.value;
    pdl.value = rest;
  }
}
// pending songs waiting for their download, at most PARALLEL at a time (a whole playlist would otherwise
// hit the extension all at once)
const PARALLEL = 5;
const pqueue = ref<{ p: Pending; ext: string; quality: string | null }[]>([]);
const queuedIds = computed(() => new Set(pqueue.value.map((q) => q.p.id)));
let pqWorkers = 0;
function queuePending(p: Pending, ext: string, quality: string | null) {
  if (p.id in pdl.value || queuedIds.value.has(p.id)) return;
  const { [p.id]: _, ...ok } = pfail.value;
  pfail.value = ok;
  pqueue.value = [...pqueue.value, { p, ext, quality }];
  runPending();
}
function runPending() {
  for (; pqWorkers < PARALLEL && pqueue.value.length; pqWorkers++) {
    (async () => {
      while (pqueue.value.length) {
        const [job, ...rest] = pqueue.value;
        pqueue.value = rest;
        await downloadPending(job.p, job.ext, job.quality);
      }
      pqWorkers--;
    })();
  }
}
// a queued song leaves the queue; a running download is stopped by the backend
function cancelPending(p: Pending) {
  pqueue.value = pqueue.value.filter((q) => q.p.id !== p.id);
  if (p.id in pdl.value) invoke("ext_cancel", { key: `pending:${p.id}` });
}
function cancelAll() {
  pqueue.value = [];
  Object.keys(pdl.value).forEach((id) => invoke("ext_cancel", { key: `pending:${id}` }));
}
const downloadAll = (ext: string, quality: string | null) => plPending.value.forEach((p) => queuePending(p, ext, quality));
const pendingLeft = computed(() => pqueue.value.length + Object.keys(pdl.value).length);
const pendingFailed = computed(() => plPending.value.filter((p) => p.id in pfail.value).length);
// everything downloading or waiting, for the bell
const jobs = computed<Job[]>(() => [
  ...downloads.value.map((d) => ({ key: `s3:${d.path}`, title: d.title, pct: d.pct })),
  ...Object.entries(redl.value).map(([path, pct]) => ({ key: `re:${path}`, title: rawLibrary.value.find((t) => t.path === path)?.title ?? path, pct })),
  ...Object.entries(mixing).map(([key, title]) => ({ key: `m:${key}`, title: `Mix · ${title}`, pct: null, detail: "Membuat mix…" })),
  ...Object.entries(catalogDl).map(([key, d]) => ({ key: `c:${key}`, title: d.title, pct: d.pct, detail: activity[d.ext], cancel: () => invoke("ext_cancel", { key }) })),
  ...Object.entries(pdl.value).map(([id, pct]) => ({ key: `p:${id}`, title: pdlTitle[+id] ?? "", pct, detail: activity[pdlExt[+id]], cancel: () => invoke("ext_cancel", { key: `pending:${id}` }) })),
  ...pqueue.value.map((q) => ({ key: `q:${q.p.id}`, title: q.p.title, pct: null, queued: true, cancel: () => cancelPending(q.p) })),
]);
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
  // Android has no system folder picker in Tauri: an in-app one browses the phone's storage (READ_MEDIA_AUDIO).
  // iOS apps only see their own Documents folder, which the Files app shows (Info.ios.plist).
  const android = navigator.userAgent.includes("Android");
  const ios = !android && document.documentElement.classList.contains("mobile");
  const dir = android ? await chooseFolder() : ios ? await documentDir() : await open({ directory: true });
  if (!dir) return;
  if (ios) notify("Salin lagu lewat app File › Di iPhone Saya › Neptunus, lalu pindai ulang folder ini.", "done");
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

// "Simpan saja" only stores the settings and keeps the connection state (a disconnected or new bucket
// stays unconnected); "Simpan & sinkronkan" connects.
async function s3Save(connect = false) {
  s3.value.disconnected = !(connect || s3Saved.value);
  await invoke("s3_save_config", { config: s3.value });
  flash("Tersimpan.");
  s3Saved.value = !!s3.value.bucket && !s3.value.disconnected;
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
async function s3Disconnect() {
  const ok = await confirm({
    title: `Putuskan "${s3.value.bucket}"?`,
    message: "Lagu cloud disembunyikan dari library sampai dihubungkan lagi. Pengaturan koneksi tetap disimpan, dan isi bucket tidak dihapus.",
    confirm: "Putuskan",
    icon: "cloud",
    danger: true,
  });
  if (!ok) return;
  try {
    await invoke("s3_disconnect");
    s3.value.disconnected = true;
    s3Saved.value = false;
    s3Edit.value = false;
    flash("Koneksi S3 diputuskan.");
  } catch (e) {
    s3Status.value = `Gagal: ${e}`;
  }
}
async function s3Sync() {
  s3Busy.value = true;
  s3Status.value = "Menghubungkan…";
  try {
    await s3Save(true);
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

// path of the picked song while the backend opens it (an S3 stream takes a moment); the current song
// keeps playing and showing until then
const loadingPath = ref("");
async function playAt(list: Track[], i: number) {
  if (!list[i]) return;
  if (list !== queue.value) followMore = followNext = null; // another list: what followed the old one stops
  loadingPath.value = list[i].path;
  try {
    if (!(await invoke<boolean>("play", { path: source(list[i]), volume: volume.value }))) return;
    queue.value = list;
    current.value = i;
    preloaded = -1;
    position.value = 0;
    paused.value = false;
  } finally {
    if (loadingPath.value === list[i].path) loadingPath.value = "";
  }
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
  // catalog songs follow their list in order
  if (shuffle.value && !followMore) return Math.floor(Math.random() * queue.value.length);
  if (current.value + 1 < queue.value.length) return current.value + 1;
  return repeat.value === "all" ? 0 : null;
}

// Songs that follow the queue once it runs out: the rest of a catalog list, or a playlist's songs not yet
// downloaded. The next one is fetched (streamed) while the queue's last song plays, then added to it.
let followMore: More | null = null;
let followNext: ReturnType<More> | null = null;
function follow(more: More | null) {
  followMore = more;
  followNext = null;
  prefetch();
}
function prefetch() {
  if (followMore && !followNext && current.value === queue.value.length - 1) followNext = followMore();
}
watch(current, prefetch);
async function playCatalog(t: Track, more?: More) {
  await playAt([t], 0);
  follow(more ?? null);
}

async function next(auto = false) {
  const i = nextIndex(auto);
  if (i !== null) return playAt(queue.value, i);
  const more = followMore;
  if (!more) return void (current.value = -1);
  const n = await (followNext ?? more()).catch(() => null);
  if (followMore !== more) return; // something else was played meanwhile
  if (!n) return void ((current.value = -1), (followMore = null));
  await playAt([...queue.value, n[0]], queue.value.length);
  follow(n[1]);
}

// A playlist plays on into its songs not yet downloaded, streamed with the stream extension (Settings › Ekstensi).
const pstream = ref<Record<number, true>>({});
// streamed file -> the pending song it came from, so the playing one is marked in the playlist
const streamedFrom = reactive<Record<string, number>>({});
async function streamPending(p: Pending): Promise<Track | null> {
  const id = streamExt.value;
  if (!exts.value.some((e) => e.enabled && e.id === id)) return null;
  pstream.value = { ...pstream.value, [p.id]: true };
  try {
    const track = await pendingTrack(p);
    const t = await withVerify(id, () => invoke<Track>("ext_stream", { id, key: `stream:${p.id}`, quality: streamQuality.value || null, track }));
    streamedFrom[t.path] = p.id;
    return t;
  } catch (e) {
    onDownloadFailed(p.title, id, e);
    return null;
  } finally {
    const { [p.id]: _, ...rest } = pstream.value;
    pstream.value = rest;
  }
}
const pendingMore = (list: Pending[], j: number): More => async () => {
  for (let k = j + 1; k < list.length; k++) {
    const t = await streamPending(list[k]);
    if (t) return [t, pendingMore(list, k)];
  }
  return null;
};
// the open playlist from song i: its downloaded songs, then (streamed) the rest
async function playList(i: number) {
  const pending = view.value === "playlists" && streamExt.value ? [...plPending.value] : [];
  if (shown.value.length) {
    await playAt(shown.value, i);
    follow(pending.length ? pendingMore(pending, -1) : null);
  } else if (pending.length) {
    const t = await streamPending(pending[0]);
    if (t) await playCatalog(t, pendingMore(pending, 0));
  }
}
async function playPending(j: number) {
  const pending = [...plPending.value];
  const t = await streamPending(pending[j]);
  if (t) await playCatalog(t, pendingMore(pending, j));
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
    if (loadingPath.value) return; // the next song is already on its way
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
  const path = now.value ? source(now.value) : "";
  buffered.value = isS3(path) ? await invoke<number | null>("s3_buffered", { path }) : null;
  if (!loadingPath.value && preloaded < 0 && st.queued === 1 && now.value && now.value.duration_secs - st.position_secs < 10) {
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
    if (c) invoke("media_cover", { path }).catch(() => {});
    const seed = c ? await seedFromImage(c).catch(() => DEFAULT_SEED) : DEFAULT_SEED;
    if (now.value?.path === path) applyTheme(seed);
  });
});

watch([lyricIdx, showNow], () => {
  if (lyricIdx.value >= 0 && showNow.value)
    document.querySelector(".lyrics .cur")?.scrollIntoView({ block: "center", behavior: "smooth" });
}, { flush: "post" });

// OS media keys / MPRIS / SMTC; on Android the MediaService notification (see MainActivity)
const android = (window as any).NeptunusAndroid as
  | { meta(t: string, a: string, al: string, secs: number): void; state(s: number, secs: number): void }
  | undefined;
watch(now, (n) => {
  if (!n) return;
  invoke("media_metadata", {
    path: n.path, title: n.title, artist: n.artist, album: n.album, durationSecs: n.duration_secs,
  });
  android?.meta(n.title, n.artist, n.album, n.duration_secs);
});
function reportPlayback() {
  invoke("media_playback", { playing: current.value < 0 ? null : !paused.value, position: position.value });
  android?.state(current.value < 0 ? -1 : paused.value ? 0 : 1, position.value);
}
watch([current, paused], reportPlayback, { immediate: true });

function mediaKey(key: string) {
  if (key === "next") next();
  else if (key === "prev") prev();
  else if (key === "toggle" || (key === "play") === paused.value) togglePause();
}
async function mediaSeek(secs: number) {
  await invoke("seek", { secs });
  position.value = secs;
  reportPlayback();
}
Object.assign(window, { __mediaKey: mediaKey, __mediaSeek: mediaSeek });

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape" && addMenu.value) return void (addMenu.value = null);
  const el = e.target as HTMLElement;
  if (el.closest("input, select, textarea, button")) return;
  if (e.code === "Space") { e.preventDefault(); togglePause(); }
  else if (e.key === "Escape") showNow.value = false;
}

const subs: Promise<UnlistenFn>[] = [];
onMounted(async () => {
  firstRun();
  window.addEventListener("keydown", onKey);
  window.addEventListener("pointerdown", closeMenu);
  invoke<ExtInfo[]>("ext_list").then((l) => (exts.value = l));
  if (view.value === "playlists" && plId.value !== null) openPlaylist(plId.value);
  // phones: the verification page replaced the app, and the grant it left behind is finished here
  if (document.documentElement.classList.contains("mobile"))
    invoke<string | null>("ext_session_poll", { text: null }).then(
      (id) => id && (verified.value++, notify("Verifikasi selesai, silakan unduh ulang lagunya.", "done")),
      (e) => notify(String(e)),
    );
  subs.push(
    listen("player-tick", poll),
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
    listen<string>("media-key", ({ payload }) => mediaKey(payload)),
    listen<number>("media-seek", ({ payload }) => mediaSeek(payload)),
  );
  applyTheme(DEFAULT_SEED);
  await restorePlayer();
  rawLibrary.value = await invoke<Track[]>("load_library");
  await refreshPlaylists();
  s3.value = await invoke<S3Config>("s3_get_config");
  s3Saved.value = !!s3.value.bucket && !s3.value.disconnected;
  await refreshFolders();
  checkUpdate();
});
onUnmounted(() => {
  window.removeEventListener("keydown", onKey);
  window.removeEventListener("pointerdown", closeMenu);
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
  reportPlayback();
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
  <FolderPicker />
  <Onboarding @done="$event === 'folder' ? pickFolder() : setView('catalog')" />
  <TitleBar :no-settings="watching" @settings="openSettings('about')">
    <Notifications ref="bell" :jobs="jobs" :notes="notes" :unread="unread" @read="unread = 0" @clear="notes = []" @cancel-all="cancelAll" @open="openUrl" />
  </TitleBar>

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

    <main :class="{ 'has-player': !!now }" :style="{ '--top-h': `${topH}px` }">
      <header ref="topEl" class="top">
        <div class="heading">
          <button v-if="group !== null" class="icon-btn tonal" @click="group = null" title="Kembali"><Icon name="back" /></button>
          <div>
            <h1>{{ title }}</h1>
            <p v-if="(library.length || view === 'videos') && view !== 's3'" class="sub">{{ subtitle }}</p>
          </div>
        </div>
        <label v-if="(library.length || (view === 'catalog' && extCatalog) || view === 'videos') && view !== 's3' && !(view === 'videos' && videoDetail)" class="search">
          <Icon name="search" />
          <input :placeholder="view === 'videos' ? 'Cari video' : 'Cari lagu, artis, album'" v-model="query" />
          <button v-if="query" class="icon-btn" @click="query = ''" title="Hapus pencarian"><Icon name="close" :size="20" /></button>
        </label>
        <div v-if="showingList && (view !== 'playlists' || plId !== null) && (shown.length || (view === 'playlists' && plPending.length))" class="btn-group actions">
          <template v-if="shown.length || (view === 'playlists' && streamExt)">
            <button class="btn filled big" @click="playList(0)"><Icon name="play" :size="28" />Putar</button>
            <button class="btn tonal big" @click="playShuffled"><Icon name="shuffle" />Acak</button>
          </template>
          <SongMenu v-if="view === 'playlists' && plPending.length" download title="semua lagu yang belum diunduh" :downloaders="downloaders"
            :label="pendingLeft ? `Mengunduh… ${pendingLeft} tersisa` : pendingFailed ? `Coba lagi yang gagal (${pendingFailed})` : `Unduh semua (${plPending.length})`"
            @redownload="downloadAll" />
          <button v-if="view === 'playlists' && pendingLeft" class="btn outlined big" @click="cancelAll"><Icon name="close" />Batalkan</button>
        </div>
        <div v-if="view === 's3'" class="tabs" role="tablist">
          <button v-for="t in SETTINGS_TABS" :key="t.id" class="tab" :class="{ sel: settingsTab === t.id }" role="tab"
            :aria-selected="settingsTab === t.id" @click="settingsTab = t.id"><span class="tab-c"><Icon :name="t.icon" :size="20" />{{ t.label }}</span></button>
        </div>
      </header>

      <p v-if="loading" class="scanning"><span class="spinner" :style="{ clipPath: 'url(#shape-cookie12)' }"></span>Memindai folder musik…</p>

      <VideoLibrary v-if="view === 'videos'" :home="videoHome" :query="query" :exts="exts" @play="current >= 0 && !paused && togglePause()" @count="videoCount = $event" @exts="exts = $event" @detail="videoDetail = $event" @watching="watching = $event" @notice="notify($event)" />
      <section v-else-if="!library.length && view !== 's3' && !(view === 'catalog' && extCatalog)" class="empty">
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
        <template v-else-if="settingsTab === 'cache'">
          <section class="panel conn cache-card">
            <div class="conn-head">
              <button class="icon-btn fold" :class="{ open: cacheOpen.film }" :aria-expanded="cacheOpen.film" :title="cacheOpen.film ? 'Tutup' : 'Lihat isi'" @click="cacheOpen.film = !cacheOpen.film"><Icon name="chevron" /></button>
              <span class="conn-mark" :style="{ clipPath: 'url(#shape-cookie9)' }"><Icon name="movie" :size="22" /></span>
              <div class="conn-id">
                <strong>Cache film</strong>
                <span>{{ cacheSize === null ? "Menghitung…" : `${bytes(cacheSize)} dipakai untuk buffer film online dan poster katalog` }}</span>
              </div>
              <button class="btn tonal" :disabled="!cacheSize" @click="clearCache"><Icon name="trash" />Hapus cache</button>
            </div>
            <div v-if="cacheOpen.film && cacheFilms.length" class="cfilms">
              <div class="cf-head">
                <button class="cf-all" @click="pickAll">
                  <Icon :name="cachePick.size === cacheFilms.length ? 'checkOn' : 'checkOff'" :size="20" />Pilih semua
                </button>
                <button class="btn filled" :disabled="!cachePick.size" @click="removePicked">
                  <Icon name="trash" />{{ cachePick.size ? `Hapus ${cachePick.size} (${bytes(pickedSize)})` : "Hapus yang dipilih" }}
                </button>
              </div>
              <ul class="cf-list">
                <li v-for="f in cacheFilms" :key="f.key">
                  <button class="cf-row" role="checkbox" :aria-checked="cachePick.has(f.key)" :class="{ on: cachePick.has(f.key) }" @click="togglePick(f.key)">
                    <Icon :name="cachePick.has(f.key) ? 'checkOn' : 'checkOff'" :size="22" class="cf-box" />
                    <span class="cf-text">
                      <strong>{{ f.title ?? "Tanpa judul" }}</strong>
                      <span>{{ [`${Math.round(f.done * 100)}% terunduh`, playedOn(f.used) && `diputar ${playedOn(f.used)}`].filter(Boolean).join(" · ") }}</span>
                    </span>
                    <span class="cf-size">{{ bytes(f.size) }}</span>
                  </button>
                </li>
              </ul>
            </div>
            <div v-if="cacheOpen.film" class="look-row">
              <div class="look-text"><strong>Simpan selama</strong><span>Film yang tidak diputar lebih lama dari ini dihapus otomatis. Total cache paling banyak 10 GB.</span></div>
              <Dropdown class="cache-days" label="Simpan selama" :model-value="cacheDays" :options="CACHE_DAYS" @update:model-value="setCacheDays" />
            </div>
          </section>
          <section class="panel conn cache-card">
            <div class="conn-head">
              <button class="icon-btn fold" :class="{ open: cacheOpen.music }" :aria-expanded="cacheOpen.music" :title="cacheOpen.music ? 'Tutup' : 'Lihat isi'" :disabled="!cacheSongs.length" @click="cacheOpen.music = !cacheOpen.music"><Icon name="chevron" /></button>
              <span class="conn-mark" :style="{ clipPath: 'url(#shape-cookie9)' }"><Icon name="note" :size="22" /></span>
              <div class="conn-id">
                <strong>Cache musik</strong>
                <span>{{ cacheSongs.length ? `${cacheSongs.length} lagu stream, ${bytes(songsSize)} dari paling banyak 1 GB` : "Lagu yang diputar dengan stream disimpan di sini sementara." }}</span>
              </div>
              <button class="btn tonal" :disabled="!cacheSongs.length" @click="removeSongs(true)"><Icon name="trash" />Hapus cache</button>
            </div>
            <div v-if="cacheOpen.music && cacheSongs.length" class="cfilms">
              <div class="cf-head">
                <button class="cf-all" @click="pickAllSongs">
                  <Icon :name="songPick.size === cacheSongs.length ? 'checkOn' : 'checkOff'" :size="20" />Pilih semua
                </button>
                <button class="btn filled" :disabled="!songPick.size" @click="removeSongs(false)">
                  <Icon name="trash" />{{ songPick.size ? `Hapus ${songPick.size} (${bytes(pickedSongsSize)})` : "Hapus yang dipilih" }}
                </button>
              </div>
              <ul class="cf-list">
                <li v-for="f in cacheSongs" :key="f.path">
                  <button class="cf-row" role="checkbox" :aria-checked="songPick.has(f.path)" :class="{ on: songPick.has(f.path) }" @click="toggleSong(f.path)">
                    <Icon :name="songPick.has(f.path) ? 'checkOn' : 'checkOff'" :size="22" class="cf-box" />
                    <span class="cf-text">
                      <strong>{{ f.title || "Tanpa judul" }}</strong>
                      <span>{{ [f.artist, playedOn(f.used) && `diputar ${playedOn(f.used)}`].filter(Boolean).join(" · ") }}</span>
                    </span>
                    <span class="cf-size">{{ bytes(f.size) }}</span>
                  </button>
                </li>
              </ul>
            </div>
          </section>
        </template>
        <Equalizer v-else-if="settingsTab === 'eq'" />
        <section v-else-if="settingsTab === 'look'" class="panel look">
          <div class="look-row">
            <div class="look-text"><strong>Tema</strong><span>Ikuti sistem, atau selalu terang atau gelap.</span></div>
            <div class="seg" role="radiogroup" aria-label="Tema">
              <button v-for="m in THEME_MODES" :key="m.id" role="radio" :aria-checked="themeMode === m.id" :class="{ sel: themeMode === m.id }"
                @click="setThemeMode(m.id)"><Icon :name="m.icon" :size="18" />{{ m.label }}</button>
            </div>
          </div>
          <div v-if="mpv.available" class="look-row">
            <div class="look-text"><strong>Pemutar film</strong><span>{{ mpv.label }} memutar lebih banyak format (MKV, HEVC, AC3/DTS) dan lebih tahan stream lambat{{ mpv.label === "mpv" ? ", di jendelanya sendiri" : ", dengan subtitle dari ekstensi" }}.</span></div>
            <div class="seg" role="radiogroup" aria-label="Pemutar film">
              <button v-for="m in ENGINES" :key="m.id" role="radio" :aria-checked="mpv.mode === m.id" :class="{ sel: mpv.mode === m.id }"
                @click="setEngine(m.id)"><Icon :name="m.icon" :size="18" />{{ m.label || mpv.label }}</button>
            </div>
          </div>
        </section>
        <About v-else-if="settingsTab === 'about'" />
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
              <button class="icon-btn tonal conn-edit" title="Putuskan koneksi" aria-label="Putuskan koneksi" @click="s3Disconnect" :disabled="s3Busy"><Icon name="linkOff" /></button>
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
              <button type="button" class="btn tonal" @click="s3Save()" :disabled="s3Busy"><Icon name="save" />Simpan saja</button>
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
          <button class="chip mix-chip" title="Buat playlist dari lagu atau artis" @click="mixPlaylist"><Icon name="playlist" :size="18" />Buat mix</button>
          <form class="chip new" @submit.prevent="createPlaylist">
            <Icon name="add" :size="18" />
            <input placeholder="Playlist baru / link Spotify" v-model="newPlaylist" aria-label="Nama playlist baru" />
          </form>
        </div>
        <p v-if="view === 'playlists' && plNote" class="pl-note" role="status">
          {{ plNote }}
          <button class="icon-btn small" title="Tutup" @click="plNote = ''"><Icon name="close" :size="18" /></button>
        </p>

        <div v-if="view === 'catalog'" class="catalog">
          <ExtCatalog :exts="exts" :library="visibleLibrary" :query="query" :now-path="now?.path" :playing="playing" :redl="redl"
            @downloaded="onDownloaded" @failed="onDownloadFailed" @play="playCatalog" @saved="mixSaved" @mixed="mixed" @toggle="togglePause" @redownload="redownload" @remove="removeSong" />
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
            <button class="row" @click="playList(i)">
              <span v-if="view === 'playlists'" class="no">{{ i + 1 }}</span>
              <span class="art-wrap">
                <CoverArt :path="t.path" />
                <span v-if="now?.path === t.path" class="art-overlay">
                  <span class="eq" :class="{ still: paused }" aria-label="Sedang diputar"><i></i><i></i><i></i></span>
                </span>
                <span v-else-if="loadingPath === t.path" class="art-overlay">
                  <span class="spinner" :style="{ clipPath: 'url(#shape-cookie12)' }" aria-label="Memuat"></span>
                </span>
              </span>
              <span class="meta">
                <strong><Icon v-if="isS3(t.path)" name="cloud" :size="16" class="cloud-mark" title="Di-stream dari S3" />{{ t.title }}</strong>
                <span>{{ unknown(t.artist) }} · {{ unknown(t.album) }}</span>
              </span>
            </button>
            <button v-if="isS3(t.path) && !phone" class="icon-btn row-act dl" :disabled="t.path in progress"
              :title="t.path in progress ? `Mengunduh ${progress[t.path]}%` : 'Unduh ke folder Musik'"
              :aria-label="`Unduh ${t.title}`" @click="download(t.path)">
              <span v-if="t.path in progress" class="pct">{{ progress[t.path] }}%</span>
              <Icon v-else name="download" :size="20" />
            </button>
            <template v-if="!phone">
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
            </template>
            <SongMenu class="row-act" :class="{ busy: t.path in redl }" :title="t.title" :downloaders="isS3(t.path) ? [] : downloaders"
              :busy="redl[t.path] ?? null" @redownload="(id, q) => redownload(t, id, q)" @remove="removeSong(t)">
              <template v-if="phone" #default="{ pick }">
                <button v-if="isS3(t.path)" role="menuitem" :disabled="t.path in progress" @click="pick(() => download(t.path))">
                  <Icon name="download" :size="20" />{{ t.path in progress ? `Mengunduh ${progress[t.path]}%` : "Unduh ke folder Musik" }}
                </button>
                <button v-if="view === 'playlists'" role="menuitem" @click="pick(() => removeFrom(t.path))"><Icon name="close" :size="20" />Hapus dari playlist</button>
                <template v-else-if="playlists.length">
                  <p>Tambah ke playlist</p>
                  <button v-for="p in playlists" :key="p.id" role="menuitem" @click="pick(() => addTo(p.id, t.path))"><Icon name="playlistAdd" :size="20" />{{ p.name }}</button>
                </template>
              </template>
            </SongMenu>
            <span class="dur">{{ fmt(t.duration_secs) }}</span>
          </li>
          <template v-if="view === 'playlists'">
            <li v-for="(p, j) in plPending" :key="`p${p.id}`" class="pending" :class="{ active: now && streamedFrom[now.path] === p.id }">
              <component :is="streamExt ? 'button' : 'div'" class="row" :title="streamExt ? 'Putar (stream)' : undefined" @click="streamExt && playPending(j)">
                <span class="no">{{ shown.length + j + 1 }}</span>
                <span class="art-wrap"><img v-if="p.cover" :src="p.cover" class="pend-art" alt="" loading="lazy" /><span v-else class="pend-art"><Icon name="note" :size="24" /></span>
                  <span v-if="now && streamedFrom[now.path] === p.id" class="art-overlay">
                    <span class="eq" :class="{ still: paused }" aria-label="Sedang diputar"><i></i><i></i><i></i></span>
                  </span>
                  <span v-else-if="p.id in pstream" class="art-overlay">
                    <span class="spinner" :style="{ clipPath: 'url(#shape-cookie12)' }" aria-label="Memuat"></span>
                  </span>
                </span>
                <span class="meta">
                  <strong>{{ p.title }}</strong>
                  <span v-if="p.id in pstream" class="live">Memuat stream…</span>
                  <span v-else-if="p.id in pdl" class="live">{{ activity[pdlExt[p.id]] || "Memulai…" }}</span>
                  <span v-else-if="queuedIds.has(p.id)">Dalam antrean</span>
                  <span v-else>{{ unknown(p.artist) }}<template v-if="p.album"> · {{ p.album }}</template> · belum diunduh</span>
                </span>
              </component>
              <SongMenu class="row-act" :class="{ busy: p.id in pdl || p.id in pfail }" download :title="p.title" :downloaders="downloaders"
                :busy="pdl[p.id] ?? null" :failed="pfail[p.id]"
                @redownload="(id, q) => queuePending(p, id, q)" />
              <button v-if="p.id in pdl || queuedIds.has(p.id)" class="icon-btn row-act" title="Batalkan unduhan" @click="cancelPending(p)"><Icon name="close" :size="20" /></button>
              <button v-else class="icon-btn row-act" title="Hapus dari playlist" @click="removePending(p)"><Icon name="close" :size="20" /></button>
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
      <button v-if="toastAction" class="toast-act" @click="toastAction.run(); toast = ''">{{ toastAction.label }}</button>
      <button class="icon-btn small" title="Tutup" @click="toast = ''"><Icon name="close" :size="18" /></button>
    </div>
  </Transition>
  <Transition name="pop">
    <div v-if="update" class="update" role="status">
      <span class="update-mark"><Icon name="update" :size="22" /></span>
      <div class="update-text">
        <strong>Versi {{ update.version }} tersedia</strong>
        <span>Rilis baru Neptunus sudah ada di GitHub.</span>
      </div>
      <button class="btn filled small" :disabled="updater.busy" @click="installUpdate">{{ updater.busy ? `Mengunduh ${updater.pct}%` : updater.done ? "Dibuka" : updater.error ? "Coba lagi" : "Unduh & pasang" }}</button>
      <button class="icon-btn small" title="Lewati versi ini" aria-label="Lewati versi ini" @click="skipUpdate"><Icon name="close" :size="18" /></button>
    </div>
  </Transition>
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
        <WavyProgress :value="position" :max="now.duration_secs" :buffered="buffered" :playing="playing" @scrub="scrub" @seek="seekTo" />
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
  height: calc(100dvh - var(--titlebar) - var(--sheet-gap));
  margin: var(--titlebar) var(--sheet-gap) 0; overflow: hidden;
  border-radius: var(--sheet-radius) var(--sheet-radius) 0 0; background: var(--surface);
}
.app.collapsed { grid-template-columns: 80px 1fr; }
.collapse-btn { margin-top: auto; align-self: center; }
.collapsed .lbl, .collapsed .badge { display: none; }
.collapsed .fab, .collapsed .dest-item { justify-content: center; padding: 0; }
.collapsed .fab { align-self: center; width: 56px; }
.collapsed .dest-item { width: 56px; align-self: center; border-radius: 16px; }
main { --pad-l: 16px; --pad-r: 8px; overflow-y: auto; padding: 0 var(--pad-r) 40px var(--pad-l); }
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
  display: flex; align-items: flex-end; justify-content: space-between; flex-wrap: wrap; gap: 16px 24px;
  /* spans main's side padding too, so scrolled content doesn't show beside the sticky header */
  margin: 0 calc(-1 * var(--pad-r)) 0 calc(-1 * var(--pad-l)); padding: 12px var(--pad-r) 24px var(--pad-l);
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
.btn.small { height: 36px; padding: 0 16px; border-radius: 18px; font-size: 14px; }
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
  overflow-x: auto; padding: 4px 4px 12px; margin: -4px -4px 0;
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
.row .no { flex: none; min-width: 2ch; margin-right: -4px; text-align: right; font-size: 14px; color: var(--on-surface-variant); font-variant-numeric: tabular-nums; }
li.active .row .no { color: inherit; }
.art-overlay { position: absolute; inset: 0; display: grid; place-items: center; border-radius: 12px; background: rgb(0 0 0 / 0.45); color: #fff; }
.meta { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
.meta strong { font-size: 16px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.toast {
  position: fixed; left: 50%; bottom: 24px; z-index: 26; transform: translateX(-50%); width: max-content; max-width: min(560px, calc(100vw - 32px));
  display: flex; align-items: center; gap: 12px; padding: 10px 8px 10px 16px; border-radius: 16px; font-size: 14px;
  background: var(--on-surface); color: var(--surface); box-shadow: 0 6px 20px rgb(0 0 0 / 0.3);
}
.toast.above { bottom: 128px; }
.update {
  position: fixed; top: calc(var(--titlebar) + 12px); right: 16px; z-index: 26; max-width: min(420px, calc(100vw - 32px));
  display: flex; align-items: center; gap: 12px; padding: 12px 8px 12px 12px; border-radius: 20px;
  background: var(--surface-container-highest); color: var(--on-surface); box-shadow: 0 6px 20px rgb(0 0 0 / 0.3);
}
.pop-enter-active, .pop-leave-active { transition: opacity 0.2s, transform 0.3s var(--emph); }
.pop-enter-from, .pop-leave-to { opacity: 0; transform: translateY(-8px); }
.update-mark { width: 40px; height: 40px; flex: none; display: grid; place-items: center; border-radius: 14px; background: var(--primary-container); color: var(--on-primary-container); }
.update-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; font-size: 13px; color: var(--on-surface-variant); }
.update-text strong { font-size: 15px; font-weight: 700; color: var(--on-surface); }
.toast .icon-btn { color: inherit; }
.toast-act { flex: none; padding: 8px 12px; border: 0; border-radius: 12px; background: none; color: var(--primary-container); font: inherit; font-weight: 700; cursor: pointer; }
.toast-enter-active, .toast-leave-active { transition: opacity 0.2s, transform 0.3s var(--emph); }
.toast-enter-from, .toast-leave-to { opacity: 0; transform: translate(-50%, 12px); }
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

/* ---------- tabs (segmented pills, same as the Video menu) ---------- */
.tabs {
  flex-basis: 100%; display: flex; gap: 4px; padding: 4px; border-radius: 28px; background: var(--surface-container-low);
  width: fit-content; max-width: 100%; flex-grow: 0; overflow-x: auto; scrollbar-width: none;
}
.tab {
  flex: none; height: 44px; padding: 0 18px 0 14px; border-radius: 22px; display: inline-flex; align-items: center;
  font-size: 15px; font-weight: 600; color: var(--on-surface-variant); transition: background 0.2s, color 0.2s;
}
.tab:hover { background: var(--surface-container-highest); }
.tab-c { display: inline-flex; align-items: center; gap: 8px; }
.tab.sel { background: var(--secondary-container); color: var(--on-secondary-container); }
@media (max-width: 600px) { .tab { padding: 0 14px 0 12px; font-size: 14px; } }

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
.mix-chip { gap: 6px; padding: 0 14px 0 10px; font-size: 14px; font-weight: 600; }
.chip.new { gap: 6px; padding: 0 6px 0 12px; border-style: dashed; }
.pl-note { display: flex; align-items: center; gap: 8px; margin: 0 0 12px; font-size: 14px; color: var(--on-surface-variant); }
.songs li.pending:not(.active) .row, .songs li.pending:not(.active) .dur { opacity: 0.75; }
.songs li.pending .row { cursor: default; }
.songs li.pending button.row { cursor: pointer; } /* streams when tapped */
.meta span.live { color: var(--primary); font-family: ui-monospace, monospace; font-size: 12px; }
.pend-art { width: 48px; height: 48px; display: grid; place-items: center; border-radius: 12px; background: var(--surface-container-high); color: var(--on-surface-variant); }
img.pend-art { object-fit: cover; }
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
.panel.conn, .panel.look { max-width: none; }
.look-row { display: flex; align-items: center; gap: 16px; flex-wrap: wrap; }
.look-text { flex: 1; min-width: 180px; display: flex; flex-direction: column; gap: 2px; font-size: 13px; color: var(--on-surface-variant); }
.look-text strong { font-size: 15px; font-weight: 700; color: var(--on-surface); }
.cache-days { width: min(100%, 220px); }
/* the Cache tab's cards: smaller than the other panels, folded open with the chevron */
.cache-card { padding: 16px 20px; gap: 12px; }
.cache-card .conn-head { gap: 12px; }
.cache-card .conn-mark { width: 40px; height: 40px; }
.cache-card .cf-list { max-height: 240px; }
.fold :deep(svg) { transition: transform 0.2s; }
.fold.open :deep(svg) { transform: rotate(90deg); }
.cfilms { display: flex; flex-direction: column; gap: 8px; }
.cf-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; flex-wrap: wrap; }
.cf-all { display: inline-flex; align-items: center; gap: 8px; height: 40px; padding: 0 12px 0 8px; border-radius: 20px; font: inherit; font-size: 14px; font-weight: 600; color: var(--on-surface); }
.cf-all:hover { background: var(--surface-container-highest); }
.cf-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; border-radius: 20px; overflow: hidden; background: var(--surface-container); max-height: 360px; overflow-y: auto; }
.cf-row { width: 100%; display: flex; align-items: center; gap: 12px; padding: 10px 16px 10px 12px; text-align: left; font: inherit; color: var(--on-surface); transition: background-color 0.2s; }
.cf-row:hover { background: var(--surface-container-high); }
.cf-row.on { background: var(--secondary-container); color: var(--on-secondary-container); }
.cf-box { flex: none; color: var(--on-surface-variant); }
.cf-row.on .cf-box { color: var(--primary); }
.cf-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.cf-text strong { font-size: 14px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.cf-text span { font-size: 12px; opacity: 0.75; }
.cf-size { flex: none; font-size: 13px; font-weight: 600; font-variant-numeric: tabular-nums; }
.seg { display: inline-flex; border: 1px solid var(--outline); border-radius: 20px; overflow: hidden; }
.seg button { display: inline-flex; align-items: center; gap: 6px; height: 40px; padding: 0 14px; font: inherit; font-size: 14px; font-weight: 600; color: var(--on-surface); background: transparent; }
.seg button + button { border-left: 1px solid var(--outline); }
.seg button.sel { background: var(--secondary-container); color: var(--on-secondary-container); }
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
.eq-bands input { writing-mode: vertical-lr; direction: rtl; height: 110px; width: 16px; accent-color: var(--primary); cursor: pointer; }
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
/* mobile app and phone-width windows: the player bar already shows the song, so the sheet is all lyrics */
:global(html.mobile .now .stage) { display: none; }
:global(html.mobile .now) { grid-template-rows: minmax(0, 1fr); }
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
.ls-hits { list-style: none; margin: 0; padding: 0; overflow-y: auto; min-height: 0; display: flex; flex-direction: column; gap: 2px; }
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

/* player: in narrow windows the seek bar stacks under the controls */
@media (max-width: 1100px) {
  /* small windows: lyrics take the screen; artwork and title go (the player bar shows the song) */
  .now { grid-template-columns: 1fr; grid-template-rows: minmax(0, 1fr); gap: 8px; padding: 64px 32px 132px; }
  .stage { display: none; }
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
  main { --pad-l: 8px; --pad-r: 16px; }
  .player { left: 8px; right: 8px; bottom: 8px; padding: 8px 8px 2px 10px; row-gap: 4px; }
  .transport { gap: 2px; }
  .transport > .icon-btn:not(.tonal), .side { display: none; }
  .transport .icon-btn.tonal { width: 34px; height: 34px; }
  .transport .icon-btn.tonal .icon { width: 22px; height: 22px; }
  .play { width: 44px; height: 36px; }
  .play .icon { width: 24px; height: 24px; }
  .np .art { width: 36px; height: 36px; border-radius: 10px; }
  .np { gap: 8px; }
  .meta strong { font-size: 14px; }
  .meta span { font-size: 12px; }
  .player .meta strong { font-size: 13px; }
  .player .meta span { font-size: 11px; }
  .seek > span { display: none; }
}
/* touch screens: no hover, so the row actions are always there */
@media (hover: none) {
  .row-act { opacity: 1; }
}
/* phones: bottom navigation bar instead of the rail, one-column forms, tighter panels */
@media (max-width: 600px) {
  .app, .app.collapsed { grid-template-columns: minmax(0, 1fr); }
  .drawer, .drawer.has-player {
    /* floating rounded bar, clear of the screen edges and the home indicator */
    position: fixed; left: max(12px, env(safe-area-inset-left)); right: max(12px, env(safe-area-inset-right));
    bottom: calc(12px + env(safe-area-inset-bottom)); z-index: 21; flex-direction: row; gap: 0; overflow: visible;
    padding: 6px; border-radius: 32px;
    background: var(--surface-container-high); box-shadow: 0 6px 20px rgb(0 0 0 / 0.3);
  }
  .dest { display: contents; }
  .collapse-btn { display: none; }
  .drawer .fab, .drawer .dest-item, .collapsed .fab, .collapsed .dest-item {
    flex: 1 1 0; min-width: 0; width: auto; height: 56px; padding: 0; gap: 2px; margin: 0;
    flex-direction: column; justify-content: center; align-self: auto;
    font-size: 11px; font-weight: 600; border-radius: 26px; box-shadow: none;
    background: none; color: var(--on-surface-variant);
  }
  .drawer .lbl, .collapsed .lbl { display: block; }
  .drawer .badge, .collapsed .badge { display: none; }
  .drawer .fab :deep(.icon), .drawer .dest-item :deep(.icon) { padding: 4px 18px; border-radius: 16px; transition: background 0.25s var(--emph); }
  .drawer .sel { color: var(--on-surface); }
  .drawer .sel :deep(.icon) { background: var(--secondary-container); color: var(--on-secondary-container); }
  main { --pad-l: max(16px, env(safe-area-inset-left)); --pad-r: max(16px, env(safe-area-inset-right)); padding-bottom: calc(100px + env(safe-area-inset-bottom)); }
  main.has-player { padding-bottom: calc(176px + env(safe-area-inset-bottom)); }
  /* the player sits just above the nav bar: one floating stack, a small gap and tighter inner corners between them */
  .player {
    bottom: calc(84px + env(safe-area-inset-bottom)); left: max(12px, env(safe-area-inset-left)); right: max(12px, env(safe-area-inset-right));
    /* no surface-coloured mask ring here: the stack floats over the list */
    border-radius: 20px 20px 8px 8px; box-shadow: 0 6px 20px rgb(0 0 0 / 0.25);
  }
  .drawer.has-player { border-radius: 8px 8px 32px 32px; }
  .toast, .toast.above { bottom: calc(164px + env(safe-area-inset-bottom)); }
  .now { padding-bottom: calc(200px + env(safe-area-inset-bottom)); }
  .fields { grid-template-columns: 1fr; }
  .panel { padding: 20px 16px; border-radius: 24px; }
  .conn-id { min-width: 0; flex-basis: calc(100% - 72px); } /* buttons wrap under the title */
  .chip.new input { width: 140px; }
  .btn.big { height: 56px; padding: 0 24px 0 18px; font-size: 16px; }
  .songs .dur { display: none; }
  .songs li { padding-right: 6px; } /* no duration column: just keep the ⋮ off the row edge */
  .songs .row-act { opacity: 1; } /* the ⋮ menu holds every row action here, so it always shows */
}
@media (prefers-reduced-motion: reduce) {
  .empty-mark, .empty-mark .icon, .eq i { animation: none; }
}
</style>
