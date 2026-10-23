import { reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";

// What each video extension may be used for, switched in Settings › Ekstensi › Video. Only the
// switched-off features are stored (one JSON setting in the app database), so new ones start on.
export type Feature = "catalog" | "streams" | "subtitles";
export const FEATURES: { id: Feature; label: string }[] = [
  { id: "catalog", label: "Katalog" },
  { id: "streams", label: "Sumber film" },
  { id: "subtitles", label: "Subtitle" },
];

const KEY = "videoFeaturesOff";
const off = reactive<Record<string, Feature[]>>({});
let loaded: Promise<void> | undefined;

export const loadFeatures = () =>
  (loaded ??= invoke<string | null>("setting_get", { key: KEY })
    .then((v) => { if (v) Object.assign(off, JSON.parse(v)); })
    .catch(() => {}));

export const featureOn = (id: string, f: Feature) => !off[id]?.includes(f);

export function setFeature(id: string, f: Feature, on: boolean) {
  off[id] = [...(off[id] ?? []).filter((x) => x !== f), ...(on ? [] : [f])];
  invoke("setting_set", { key: KEY, value: JSON.stringify(off) }).catch(() => {});
}
