import { invoke } from "@tauri-apps/api/core";
import { reactive } from "vue";

export type Quality = { format: string; bits: number | null; rate: number | null; kbps: number | null; lossless: boolean };

// Audio quality by track path, read from the file's header on first sight; null: cloud song or unreadable.
export const qualities = reactive<Record<string, Quality | null>>({});
const asked = new Set<string>();

export function loadQuality(path: string) {
  if (asked.has(path)) return;
  asked.add(path);
  invoke<Quality | null>("audio_quality", { path })
    .then((q) => (qualities[path] = q))
    .catch(() => (qualities[path] = null));
}

const khz = (hz: number) => `${+(hz / 1000).toFixed(1)}`;
/** "FLAC 24/96", "MP3 320" */
export function shortQuality(q: Quality) {
  if (q.lossless && q.bits && q.rate) return `${q.format} ${q.bits}/${khz(q.rate)}`;
  return q.kbps ? `${q.format} ${q.kbps}` : q.format;
}
/** "FLAC · 24-bit · 96 kHz · 2304 kbps" */
export function longQuality(q: Quality) {
  return [q.format, q.bits && `${q.bits}-bit`, q.rate && `${khz(q.rate)} kHz`, q.kbps && `${q.kbps} kbps`].filter(Boolean).join(" · ");
}
