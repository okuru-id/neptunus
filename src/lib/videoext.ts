import { invoke } from "@tauri-apps/api/core";
import { has, type ExtInfo } from "./ext";

// video_provider extensions bridge a Stremio addon (or similar catalog) into the video tab.
// The addon URL lives in the extension's settings; the app only talks through `ext_call`.

export type Episode = {
  id: string; name?: string; season?: number; episode?: number;
  thumbnail?: string; released?: string;
};
export type Meta = {
  id: string; name: string; type: string; poster?: string; posterShape?: string;
  releaseInfo?: string; description?: string; year?: string; videos?: Episode[];
};
export type Stream = {
  name?: string; title?: string; url?: string; infoHash?: string;
  behaviorHints?: { notWebReady?: boolean; videoSize?: number; filename?: string; videoHash?: string };
};
export type CatalogInfo = {
  id: string; name: string; type: string; extraSupported?: string[];
  extra?: { name: string; isRequired?: boolean }[]; extraRequired?: string[];
};
export type SourceInfo = { name: string; catalogs: CatalogInfo[]; streams?: boolean; subtitles?: boolean; idPrefixes?: string[] };

export const videoSources = (exts: ExtInfo[]) => exts.filter((e) => e.enabled && has(e, "video_provider"));

const call = <T,>(id: string, method: string, ...args: unknown[]) =>
  invoke<T>("ext_call", { id, method, args });

export const sourceInfo = (id: string) => call<SourceInfo>(id, "videoInfo");
export const catalog = (id: string, type: string, catId: string, extra = "") =>
  call<Meta[]>(id, "videoCatalog", type, catId, extra);
export const meta = (id: string, type: string, mid: string) => call<Meta>(id, "videoMeta", type, mid);
export const streams = (id: string, type: string, mid: string) => call<Stream[]>(id, "videoStreams", type, mid);

/** Playback risk score, lower = more likely to play in the webview. Also sorts the picker list.
 *  "pad" files (anti-hotlink) start with junk bytes, so the webview can't demux them (error 3016).
 *  Torrents (infoHash) are unusable here. */
export const streamScore = (s: Stream): number => {
  if (!s.url) return 99;
  let n = 0;
  if (s.behaviorHints?.notWebReady) n += 3; // flag ini berarti sering tak bisa diputar web
  const f = `${s.title ?? ""} ${s.behaviorHints?.filename ?? ""}`;
  if (/\|\^\||[.\s_-]pad([.\s_-]|$)/i.test(f)) n += 2; // padded: no container header at offset 0
  if ((s.behaviorHints?.videoSize ?? 0) > 8e9 || /2160p|\b4k\b|\b8k\b/i.test(f)) n += 1; // huge/4K
  if (/h[.\s]?265|hevc|x265/i.test(f)) n += 1; // h264 beats hevc when both exist
  if (!/h[.\s]?264|avc|x264/i.test(f)) n += 1; // unknown codec is riskier than h264
  return n;
};

export const pickStream = (list: Stream[]): Stream | undefined => {
  const best = [...list].sort((a, b) => streamScore(a) - streamScore(b))[0];
  return best?.url ? best : undefined;
};

/** The catalog to search in for a type: a dedicated search catalog, else one advertising `search`, else the first. */
export const searchCatalog = (cats: CatalogInfo[], type: string): CatalogInfo | undefined => {
  const of = cats.filter((c) => c.type === type);
  return of.find((c) => c.id.toLowerCase().includes("search"))
    ?? of.find((c) => (c.extraSupported ?? []).includes("search"))
    ?? of[0];
};

/** Stremio's own metadata addon: the catalog shown first when no other one is set up. */
export const CINEMETA = "https://v3-cinemeta.strem.io/manifest.json";

/** Catalogs that can be browsed as a row: no search-only ones and none that need a filter value. */
export const rowCatalogs = (info: SourceInfo) =>
  info.catalogs.filter((c) =>
    !c.id.toLowerCase().includes("search") &&
    !(c.extra ?? []).some((e) => e.isRequired) &&
    !(c.extraRequired ?? []).length);

// Whether an addon can give streams for this id;
// an addon whose manifest hasn't loaded (or failed) is still asked: better a failed tab than a missing one
export const servesStreams = (info: SourceInfo | undefined, id: string) =>
  !info || (info.streams !== false && (!info.idPrefixes?.length || info.idPrefixes.some((p) => id.startsWith(p))));

export type SourceStream = Stream & { source: string; sourceName: string };
export type StreamGroup = { id: string; name: string; list: SourceStream[]; note: string };

/** Streams for one title from every addon that serves it, one group each, so the user can play from any
 *  of them and see why an addon came back empty. */
export function allStreams(sources: { id: string; name: string; info?: SourceInfo }[], type: string, id: string) {
  return Promise.all(
    sources.filter((s) => servesStreams(s.info, id)).map(async (s): Promise<StreamGroup> => {
      try {
        const all = await streams(s.id, type, id);
        const list = all.filter((x) => x.url).map((x) => ({ ...x, source: s.id, sourceName: s.name })) as SourceStream[];
        const note = list.length ? "" : all.length ? "Hanya ada sumber torrent, belum bisa diputar di sini." : "Tidak ada sumber untuk judul ini.";
        return { id: s.id, name: s.name, list, note };
      } catch (e) {
        return { id: s.id, name: s.name, list: [], note: `Gagal memuat (${e}).` };
      }
    }),
  );
}

export type Subtitle = { id?: string; url: string; lang?: string; source: string };
/** What the player needs to ask subtitle addons for the title it plays. */
export type SubtitleQuery = { type: string; id: string; extra: string; sources: { id: string; name: string }[] };

export const subtitleQuery = (sources: { id: string; name: string; info?: SourceInfo }[], type: string, id: string, s?: Stream) => {
  const h = s?.behaviorHints;
  const extra = [
    h?.videoHash && `videoHash=${encodeURIComponent(h.videoHash)}`,
    h?.videoSize && `videoSize=${h.videoSize}`,
    h?.filename && `filename=${encodeURIComponent(h.filename)}`,
  ].filter(Boolean).join("&");
  return {
    type, id, extra,
    sources: sources.filter((x) => x.info?.subtitles && (!x.info.idPrefixes?.length || x.info.idPrefixes.some((p) => id.startsWith(p))))
      .map(({ id, name }) => ({ id, name })),
  } satisfies SubtitleQuery;
};

/** Subtitles from every subtitle addon; one failing addon doesn't hide the rest. */
export async function subtitles(q: SubtitleQuery) {
  const lists = await Promise.all(q.sources.map((s) =>
    call<Omit<Subtitle, "source">[]>(s.id, "videoSubtitles", q.type, q.id, q.extra)
      .then((l) => l.filter((x) => x.url).map((x) => ({ ...x, source: s.name })), () => [])));
  return lists.flat() as Subtitle[];
}
