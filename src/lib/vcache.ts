import { invoke } from "@tauri-apps/api/core";
import { reactive } from "vue";

// Which films and episodes have a stream in the disk cache (Rust `video_cached`). The cache is keyed by
// stream URL, so the URLs played are noted per film/episode id, and dropped once the cache lets them go.
const KEY = "neptunus.vcache";
let played: Record<string, string[]> = {};
try { played = JSON.parse(localStorage.getItem(KEY) || "{}"); } catch {}
const save = () => { try { localStorage.setItem(KEY, JSON.stringify(played)); } catch {} };

export const cached = reactive({ ids: new Set<string>(), urls: new Set<string>() });

function mark() {
  cached.ids = new Set(Object.keys(played));
  cached.urls = new Set(Object.values(played).flat());
}
mark();

/** Notes that `url` is played for the films/episodes `ids`, so they show as cached. */
export function remember(ids: string[], url: string) {
  for (const id of ids) played[id] = [...new Set([...(played[id] ?? []), url])];
  save();
  mark();
}

/** Asks which noted streams are still on disk. */
export async function refreshCached() {
  const all = [...new Set(Object.values(played).flat())];
  if (!all.length) return;
  const kept = new Set(await invoke<string[]>("video_cached", { urls: all }).catch(() => all));
  for (const [id, urls] of Object.entries(played)) {
    const left = urls.filter((u) => kept.has(u));
    if (left.length) played[id] = left;
    else delete played[id];
  }
  save();
  mark();
}
