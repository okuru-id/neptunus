import { invoke } from "@tauri-apps/api/core";

// A catalog entry as metadata-provider extensions give it.
export type Item = {
  id: string; name: string; artists?: string; album_name?: string; cover_url?: string; images?: string;
  image_url?: string; duration_ms?: number; item_type?: string; type?: string; provider_id?: string;
  release_date?: string; isrc?: string; description?: string; owner?: string;
};

export const kind = (i: Item) => (i.item_type || i.type || "track").toLowerCase();
export const coverOf = (i: Item) => i.cover_url || i.images || i.image_url || "";
const low = (s?: string) => (s ?? "").trim().toLowerCase();
const artistsOf = (t: Item) => (t.artists ?? "").split(/\s*[,&;]\s*|\s+(?:feat\.?|ft\.?|x)\s+/i).map((a) => a.trim()).filter(Boolean);

/** Search results of catalog extension `ext`. */
export async function find(ext: string, q: string): Promise<Item[]> {
  const call = <T>(method: string, args: unknown[]) => invoke<T>("ext_call", { id: ext, method, args });
  let res: Item[] | null;
  try {
    res = await call<Item[]>("customSearch", [q, {}]);
  } catch (e) {
    if (!String(e).includes("not defined")) throw e;
    res = await call<Item[]>("searchTracks", [q, 30]);
  }
  // some providers wrap results ({ tracks: [...] })
  const list = Array.isArray(res) ? res : ((res as unknown as { tracks?: Item[] })?.tracks ?? []);
  return list.filter((i) => i && i.id);
}

/** A mix of up to 30 songs from `seed`, it first. The catalogs have no "similar songs", so it is the
 *  song's artists' songs plus those of the artists they most often sing with, shuffled. `from` names
 *  the artists it drew on. */
export async function buildMix(ext: string, seed: Item): Promise<{ tracks: Item[]; from: string[] }> {
  const tracks = async (q: string) => (await find(ext, q).catch(() => [])).filter((i) => kind(i) === "track");
  const own = artistsOf(seed).slice(0, 2);
  const first = (await Promise.all(own.map(tracks))).flat();
  const seen = new Set(own.map(low));
  const count = new Map<string, number>();
  for (const t of first) for (const a of artistsOf(t)) if (!seen.has(low(a))) count.set(a, (count.get(a) ?? 0) + 1);
  const friends = [...count].sort((a, b) => b[1] - a[1]).slice(0, 4).map(([a]) => a);
  const theirs = (await Promise.all(friends.map(tracks))).map((l) => l.slice(0, 6));
  const pool = [first.slice(0, 12), ...theirs].flat().sort(() => Math.random() - 0.5);
  const out: Item[] = [];
  const keys = new Set<string>();
  for (const t of [seed, ...pool]) {
    const k = `${low(t.name)}|${low(artistsOf(t)[0])}`;
    if (keys.has(k)) continue;
    keys.add(k);
    out.push(t);
    if (out.length === 30) break;
  }
  return { tracks: out, from: [...own, ...friends] };
}

/** Saves catalog songs as a new playlist: the ones in the library join it, the rest wait to be downloaded. */
export const savePlaylist = (name: string, tracks: Item[]) =>
  invoke<{ id: number; name: string }>("playlist_from", { name,
    tracks: tracks.map((t) => [t.name, t.artists ?? "", t.album_name ?? "", Math.round((t.duration_ms ?? 0) / 1000), "", coverOf(t)]) });
