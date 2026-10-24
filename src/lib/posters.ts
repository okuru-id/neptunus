import { invoke } from "@tauri-apps/api/core";
import { ref } from "vue";

// Catalog posters go through the app's loopback server, which keeps them on disk (Rust `posters`), so the
// catalogs show them offline too. Until the server's address is known, and for anything not http(s),
// the image URL is used as it is; a ref, so the posters already on screen switch over once it is.
const base = ref<string | null>(null);
invoke<string | null>("poster_base").then((b) => (base.value = b), () => {});

type Of = { id: string; name?: string; type?: string; year?: string; releaseInfo?: string; posterShape?: string };

/** What a picture is of, so extensions listing the same title share one saved copy of it: the IMDb id
 *  most Stremio addons use, else (for a film or series) its kind, name and year. `kind` keeps a title's
 *  pictures apart, and a landscape poster from one addon never stands in for another's portrait one.
 *  Empty when nothing is known for sure (an episode without an IMDb id): the URL is the key then. */
function titleKey(m: Of, kind: string) {
  if (kind === "poster" && m.posterShape === "landscape") kind = "wide";
  const imdb = m.id.match(/^tt\d+(:\d+:\d+)?$/)?.[0];
  if (imdb) return `${imdb}:${kind}`;
  const year = (m.year || m.releaseInfo || "").match(/\d{4}/)?.[0] ?? "";
  return m.name && m.type ? `${m.type}:${m.name.trim().toLowerCase()}:${year}:${kind}` : "";
}

/** The address to show the picture at `url` of title `of` (its poster, backdrop or episode thumbnail). */
export const poster = (url?: string | null, of?: Of, kind = "poster") =>
  url && base.value && /^https?:\/\//.test(url)
    ? `${base.value}${encodeURIComponent(of ? titleKey(of, kind) : "")}/${encodeURIComponent(url)}`
    : url ?? undefined;
