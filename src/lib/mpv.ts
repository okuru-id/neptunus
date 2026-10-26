import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { reactive } from "vue";
import { byLang, subLabels, subtitles, type Subtitle, type SubtitleQuery } from "./videoext";
import { choose } from "./ConfirmDialog.vue";

// Films may play outside the built-in player: in the system's mpv on Linux (Rust `video::mpv`), in a
// player app on Android (VLC, MX Player… through the app's bridge) or on iOS (VLC or Infuse, by their
// x-callback URLs). Which one is asked each time by default, or fixed in Settings › Tampilan; only when
// there is one. `mpv` is that outside player, whatever it is here; `label` names it.
export type Engine = "ask" | "builtin" | "mpv";
const KEY = "neptunus.videoEngine";
export const mpv = reactive({
  available: false,
  label: "mpv",
  mode: (() => { try { return (localStorage.getItem(KEY) as Engine | null) ?? "ask"; } catch { return "ask"; } })() as Engine,
});
const droid = (window as any).NeptunusAndroid as { hasVideoPlayer?(): boolean; openVideo?(json: string): boolean; playNative?(json: string): void } | undefined;
let iosApp: "vlc" | "infuse" | null = null;
let iosNative = false;
/** Phones: the built-in player is the app's own screen (`playInMpv(…, true)`: ExoPlayer on Android,
 *  VLCKit on iOS), not the WebView's <video>, which plays no MKV on iOS and little AC3 or HEVC on Android. */
export const nativePlayer = () => !!droid?.playNative || iosNative;
// phones: one installed later counts too
function detect() {
  if (droid?.hasVideoPlayer) return void Object.assign(mpv, { available: droid.hasVideoPlayer(), label: "Pemutar lain" });
  invoke<boolean>("ios_native_available").then((a) => (iosNative = a), () => {});
  invoke<("vlc" | "infuse")[]>("ios_players").then(
    ([app]) => { iosApp = app ?? null; Object.assign(mpv, { available: !!app, label: app === "infuse" ? "Infuse" : "VLC" }); },
    () => invoke<boolean>("mpv_available").then((a) => (mpv.available = a), () => {}), // no such command: desktop
  );
}
detect();
const phone = () => !!droid?.openVideo || iosApp !== null;

export function setEngine(mode: Engine) {
  mpv.mode = mode;
  try { localStorage.setItem(KEY, mode); } catch {}
}

/** The player for the film at `path` about to start; null when the question was dismissed. Phones hand
 *  other apps only streams (http): a file here is the built-in player's. */
export async function pickEngine(path: string): Promise<"builtin" | "mpv" | null> {
  if (droid) detect();
  if (!mpv.available || mpv.mode === "builtin" || (phone() && !/^https?:\/\//.test(path))) return "builtin";
  if (mpv.mode === "mpv") return "mpv";
  const pick = await choose({
    title: "Putar di mana?",
    message: phone()
      ? `Pemutar aplikasi memakai kontrol Neptunus. ${mpv.label} memutar lebih banyak format (MKV, HEVC, AC3/DTS), dengan subtitle dari ekstensi, tapi posisi tontonan tidak tersimpan. Bisa diatur tetap di Pengaturan › Tampilan.`
      : "Pemutar aplikasi memakai kontrol Neptunus. mpv memutar lebih banyak format (MKV, HEVC, AC3/DTS) di jendela terpisah. Bisa diatur tetap di Pengaturan › Tampilan.",
    cancel: "Di aplikasi", confirm: mpv.label, icon: "movie",
  });
  return pick === null ? null : pick ? "mpv" : "builtin";
}

/** A stream (not this app's loopback server: on iOS it stops once the app is in the background) with
 *  its host's headers and the extensions' subtitles, to the player app. */
async function inPhoneApp(f: Film, start: number) {
  if (!/^https?:\/\//.test(f.path)) throw new Error(`${mpv.label} hanya untuk film dari ekstensi.`);
  const subs = await subsFor(f);
  if (droid?.openVideo) {
    const json = { url: f.path, title: f.title, headers: f.headers ?? {}, subs: subs.list.map(({ s, label }) => ({ url: s.url, name: label })), subOn: subs.on, position: Math.round(start * 1000) };
    if (!droid.openVideo(JSON.stringify(json))) throw new Error("Belum ada aplikasi pemutar video. Pasang VLC atau MX Player dulu.");
    return;
  }
  // one subtitle only: the pick, else the first in the usual language
  const e = encodeURIComponent;
  const sub = subs.list[0] ? `&sub=${e(subs.list[0].s.url)}` : "";
  const url = iosApp === "infuse" ? `infuse://x-callback-url/play?url=${e(f.path)}${sub}` : `vlc-x-callback://x-callback-url/stream?url=${e(f.path)}${sub}`;
  await invoke("ios_play_in", { url });
}

type Film = { path: string; title: string; sub?: SubtitleQuery; resume?: string; headers?: Record<string, string> };
type Closed = { path: string; pos: number; duration: number; eof: boolean; error: string | null };
const finished = (t: number, d: number) => d > 0 && (d - t < 10 || t / d > 0.95);

/** The extensions' subtitles for `f`, for an outside player, named like in the built-in player's menu:
 *  the one it would show first (chosen last time, or one in the last language), then the others in that
 *  language, then the rest by language. `on`: the first one is that pick, to be shown. */
export async function subsFor(f: Film): Promise<{ list: { s: Subtitle; label: string }[]; on: boolean }> {
  if (!f.sub?.sources.length) return { list: [], on: false };
  const [list, url, lang] = await Promise.all([
    subtitles(f.sub).catch(() => [] as Subtitle[]),
    invoke<string | null>("setting_get", { key: `subPick:${f.path}` }).catch(() => null),
    invoke<string | null>("setting_get", { key: "subLang" }).catch(() => null),
  ]);
  const pick = list.find((s) => s.url === url) ?? (lang !== null ? list.find((s) => (s.lang ?? "") === lang) : undefined);
  const same = (s: Subtitle) => lang !== null && (s.lang ?? "") === lang;
  // numbered in the menu's order, so "Inggris 2 - …" is the same file in both
  const named = new Map(subLabels([...list].sort(byLang)).map((x) => [x.s, x.label]));
  const rest = list.filter((s) => s !== pick).sort((a, b) => Number(same(b)) - Number(same(a)) || byLang(a, b));
  return { list: (pick ? [pick, ...rest] : rest).map((s) => ({ s, label: named.get(s)! })), on: !!pick };
}

/** Plays `f` in the outside player (or, `native`, Android's ExoPlayer screen) from where it was left.
 *  mpv and ExoPlayer keep the place like the built-in player (`video-pos:`), and `done` runs once they
 *  close, with how far it played, or why it couldn't; a phone's player app says nothing back, so there
 *  `done` never runs. */
export async function playInMpv(f: Film, done: (seconds: number, error: string | null) => void, native = false) {
  const id = f.resume ?? f.path;
  const [posKey, durKey] = [`video-pos:${id}`, `video-dur:${id}`];
  let start = 0;
  try { start = Number(localStorage.getItem(posKey) ?? localStorage.getItem(`video-pos:${f.path}`)) || 0; } catch {}
  try { if (finished(start, Number(localStorage.getItem(durKey)) || 0)) start = 0; } catch {}
  if (phone() && !native) return inPhoneApp(f, start);
  const [url, subs] = await Promise.all([
    invoke<string | null>("video_url", { path: f.path, headers: f.headers ?? null, title: f.title }).then((u) => u ?? f.path),
    subsFor(f),
  ]);
  const closed = (c: Closed) => {
    try {
      if (c.duration && !c.eof && !finished(c.pos, c.duration)) {
        localStorage.setItem(posKey, String(c.pos));
        localStorage.setItem(durKey, String(c.duration));
      } else if (c.duration) {
        localStorage.removeItem(posKey);
        localStorage.removeItem(durKey);
      }
    } catch {}
    invoke("video_stop", { url }).catch(() => {});
    done(c.pos, c.error);
  };
  if (native && droid?.playNative) {
    (window as any).__nativeVideoClosed = (c: Omit<Closed, "path">) => closed({ ...c, path: f.path });
    const list = subs.list.map(({ s, label }) => ({ url: s.url, lang: s.lang ?? "", name: label }));
    droid.playNative(JSON.stringify({ url, title: f.title, headers: f.headers ?? {}, subs: list, subOn: subs.on, position: Math.round(start * 1000) }));
    return;
  }
  if (native) {
    const off = await listen<Omit<Closed, "path">>("native-video-closed", ({ payload: c }) => {
      off();
      // the video took the audio session: the music output it stopped has to be opened again
      invoke("audio_after_video").catch(() => {});
      closed({ ...c, path: f.path });
    });
    const film = JSON.stringify({ url, title: f.title, headers: f.headers ?? {}, position: Math.round(start * 1000) });
    const list = subs.list.map(({ s, label }) => ({ url: s.url, title: label, lang: s.lang ?? "" }));
    await invoke("ios_play_native", { film, subs: list, subOn: subs.on }).catch((e) => { off(); throw e; });
    return;
  }
  const off = await listen<Closed>("mpv-closed", ({ payload: c }) => {
    if (c.path !== f.path) return;
    off();
    closed(c);
  });
  try {
    await invoke("mpv_play", { path: f.path, url, title: f.title, start, subs: subs.list.map(({ s, label }) => ({ url: s.url, title: label, lang: s.lang ?? "" })), subOn: subs.on, headers: f.headers ?? null });
  } catch (e) {
    off();
    throw e;
  }
}
