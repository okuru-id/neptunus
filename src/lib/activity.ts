import { listen } from "@tauri-apps/api/event";
import { reactive } from "vue";

// What each extension reported last (a step, a log line, a request), by extension id. Shown under a
// song while it downloads, so a download sitting at 0% still visibly works.
export const activity = reactive<Record<string, string>>({});
listen<{ ext: string; text: string }>("ext-activity", ({ payload }) => {
  activity[payload.ext] = payload.text;
});
