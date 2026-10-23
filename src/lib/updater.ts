import { reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Downloading the new installer, shared by the update banner and Settings › Tentang so both show the
// same progress. On desktop the file lands in Downloads and opens; on phones the browser takes it.
export const updater = reactive<{ busy: boolean; pct: number; done: string; error: string }>({ busy: false, pct: 0, done: "", error: "" });

export async function installUpdate() {
  if (updater.busy) return;
  Object.assign(updater, { busy: true, pct: 0, done: "", error: "" });
  const off = await listen<{ key: string; pct: number }>("release-download", ({ payload }) => {
    if (payload.key === "update") updater.pct = payload.pct;
  });
  try {
    updater.done = await invoke<string>("update_install");
  } catch (e) {
    updater.error = String(e);
  } finally {
    off();
    updater.busy = false;
  }
}
