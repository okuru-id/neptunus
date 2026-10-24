import { reactive } from "vue";
import type { Episode, Meta, SourceStream } from "./videoext";

// The viewer's own corner of the video catalogs: titles bookmarked, and the ones watched last with the
// episode and source they were played from, so the catalog can offer them first and resume them.
// Kept in localStorage: small, per device, and none of it is worth a database round trip.

/** A title as a catalog row shows it, plus the addon it was opened from. */
export type Saved = Pick<Meta, "id" | "name" | "type" | "poster" | "posterShape" | "background" | "releaseInfo" | "year"> & { source: string; at: number };
export type Watched = Saved & { episode?: Pick<Episode, "id" | "name" | "season" | "episode">; stream: SourceStream; title: string };

const BOOKMARKS = "neptunus.vbookmarks";
const HISTORY = "neptunus.vhistory";
const MAX_HISTORY = 30;

const load = <T>(key: string): T[] => { try { return JSON.parse(localStorage.getItem(key) || "[]"); } catch { return []; } };
const store = (key: string, v: unknown) => { try { localStorage.setItem(key, JSON.stringify(v)); } catch {} };

// `seen` moves when a player closes: the positions it saved live in localStorage, which Vue doesn't watch
export const vlib = reactive({ bookmarks: load<Saved>(BOOKMARKS), history: load<Watched>(HISTORY), seen: 0 });

const lite = (m: Meta, source: string): Saved => ({
  id: m.id, name: m.name, type: m.type, poster: m.poster, posterShape: m.posterShape, background: m.background,
  releaseInfo: m.releaseInfo, year: m.year, source, at: Date.now(),
});

export const isBookmarked = (id: string) => vlib.bookmarks.some((b) => b.id === id);

export function toggleBookmark(m: Meta, source: string) {
  vlib.bookmarks = isBookmarked(m.id) ? vlib.bookmarks.filter((b) => b.id !== m.id) : [lite(m, source), ...vlib.bookmarks];
  store(BOOKMARKS, vlib.bookmarks);
}

/** Notes a play: one entry per title (the latest episode of a series), newest first. */
export function recordPlay(m: Meta, source: string, ep: Episode | null, stream: SourceStream, title: string) {
  const entry: Watched = {
    ...lite(m, source), title, stream,
    episode: ep ? { id: ep.id, name: ep.name, season: ep.season, episode: ep.episode } : undefined,
  };
  vlib.history = [entry, ...vlib.history.filter((h) => h.id !== m.id)].slice(0, MAX_HISTORY);
  store(HISTORY, vlib.history);
}

export function forget(id: string) {
  vlib.history = vlib.history.filter((h) => h.id !== id);
  store(HISTORY, vlib.history);
}

export const lastPlayed = (id: string) => vlib.history.find((h) => h.id === id);

/** The key a film or episode's resume position is kept under, whichever source plays it. */
export const resumeKey = (h: Pick<Watched, "id" | "episode">) => h.episode?.id ?? h.id;

/** Where the player left `key`: seconds in and the length (VideoPlayer writes both). */
export function progressOf(key: string): { pos: number; dur: number } | null {
  try {
    const pos = Number(localStorage.getItem(`video-pos:${key}`));
    const dur = Number(localStorage.getItem(`video-dur:${key}`));
    return pos > 0 ? { pos, dur } : null;
  } catch {
    return null;
  }
}

export const clock = (t: number) => {
  const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), s = Math.floor(t % 60);
  return `${h ? `${h}:${String(m).padStart(2, "0")}` : m}:${String(s).padStart(2, "0")}`;
};
