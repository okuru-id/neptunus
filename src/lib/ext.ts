import { ref, watch } from "vue";

// Installed extensions as `ext_list` returns them: the manifest plus a few runtime fields.
export type ExtSetting = {
  key: string; type: string; label: string; description?: string; secret?: boolean;
  default?: unknown; options?: string[]; action?: string;
};
export type ExtInfo = {
  id: string; name: string; displayName?: string; version: string; description?: string; author?: string;
  type: string[]; enabled: boolean; iconData: string | null; values: Record<string, unknown>;
  settings?: ExtSetting[]; qualityOptions?: { id: string; label: string; description?: string }[];
  capabilities?: { homeFeed?: boolean };
  searchBehavior?: { enabled?: boolean; placeholder?: string };
  requiredRuntimeFeatures?: string[];
  /** the gateway wants a browser verification before downloads (SpotiFLAC signed sessions) */
  signedSession?: unknown;
};
// a repository entry (registry.json)
export type RepoExt = {
  id: string; name?: string; display_name?: string; displayName?: string; version: string; description?: string;
  download_url?: string; downloadUrl?: string; icon_url?: string; iconUrl?: string; category?: string;
  sha256?: string; checksum_sha256?: string;
};

/** The one download extension songs from the catalog stream with (played without being kept), or "". */
export const streamExt = ref((() => { try { return localStorage.getItem("neptunus.streamExt") ?? ""; } catch { return ""; } })());
watch(streamExt, (id) => { try { localStorage.setItem("neptunus.streamExt", id); } catch {} });
/** The quality it streams in ("": the extension's first, usually the best and largest). */
export const streamQuality = ref((() => { try { return localStorage.getItem("neptunus.streamQuality") ?? ""; } catch { return ""; } })());
watch(streamQuality, (q) => { try { localStorage.setItem("neptunus.streamQuality", q); } catch {} });
watch(streamExt, () => (streamQuality.value = ""));

type Track = { path: string; title: string; artist: string; album: string; duration_secs: number };
/** The song after one played from a catalog list, with what comes after it in turn. */
export type More = () => Promise<[Track, More] | null>;

export const extName = (e: ExtInfo) => e.displayName || e.name;
export const has = (e: ExtInfo, t: string) => e.type?.includes(t);
// host features known to be missing here (names before the "@"); patternedFileTransform is supported now
const MISSING: string[] = [];
/** Needs host features this app does not provide. */
export const unsupported = (e: { requiredRuntimeFeatures?: string[] }) =>
  (e.requiredRuntimeFeatures ?? []).filter((f) => MISSING.includes(f.split("@")[0]));

/** "1.10.2" vs "1.9.0" */
export function newer(a: string, b: string) {
  const pa = a.split(".").map(Number), pb = b.split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] || 0) - (pb[i] || 0);
    if (d) return d > 0;
  }
  return false;
}

/** A failed download's error, said plainly: `song` could not be downloaded with extension `ext`. */
export function downloadError(err: unknown, song: string, ext: string): string {
  const e = String(err);
  if (/not found|no .*match|not available|unavailable|tidak ditemukan/i.test(e)) return `“${song}” tidak tersedia di ${ext}. Coba ekstensi lain.`;
  if (/verif/i.test(e)) return `Verifikasi ${ext} belum selesai, jadi “${song}” tidak diunduh.`;
  if (/\b429\b|rate.?limit|too many/i.test(e)) return `${ext} sedang membatasi permintaan. Tunggu beberapa menit, lalu coba lagi.`;
  if (/took too long|timed? ?out/i.test(e)) return `${ext} terlalu lama merespons. Coba lagi nanti atau pakai ekstensi lain.`;
  if (/\b40[13]\b|forbidden|unauthori[sz]ed/i.test(e)) return `${ext} menolak permintaan. Coba lagi nanti atau pakai ekstensi lain.`;
  if (/dns|resolve|connect|network|unreachable/i.test(e)) return `Tidak bisa terhubung ke ${ext}. Periksa koneksi internet.`;
  if (/cancel/i.test(e)) return `Unduhan “${song}” dibatalkan.`;
  return `Gagal mengunduh “${song}” dengan ${ext}: ${e.replace(/^Error:\s*/, "")}`;
}
