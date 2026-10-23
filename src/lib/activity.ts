import { listen } from "@tauri-apps/api/event";
import { reactive } from "vue";

// What each extension reported last (a step, a log line, a request), by extension id. Shown under a
// song while it downloads, so a download sitting at 0% still visibly works.
export const activity = reactive<Record<string, string>>({});
listen<{ ext: string; text: string }>("ext-activity", ({ payload }) => {
  activity[payload.ext] = payload.text;
});

// catalog downloads by key, kept here so the bell lists them after the catalog is left
export const catalogDl = reactive<Record<string, { title: string; ext: string; pct: number }>>({});
listen<{ key: string; pct: number }>("ext-download", ({ payload }) => {
  if (catalogDl[payload.key]) catalogDl[payload.key].pct = payload.pct;
});
