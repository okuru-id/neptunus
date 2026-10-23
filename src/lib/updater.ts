import { reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Downloading the new installer, shared by the update banner and Settings › Tentang so both show the
// same progress. On desktop the file lands in Downloads and installs; Android installs the APK from
// the app; iOS installs over the air when the release is signed.
export const updater = reactive<{ busy: boolean; pct: number; done: string; error: string }>({ busy: false, pct: 0, done: "", error: "" });

const droid = (window as any).NeptunusAndroid as { installApk?(path: string): boolean } | undefined;
/** Android: an `apk:<path>` result goes to the system installer. "apk" once it shows, "apk-permission"
 *  when installing from this app has to be allowed first (its settings page opens). */
export function openApk(result: string) {
  if (!result.startsWith("apk:") || !droid?.installApk) return result;
  return droid.installApk(result.slice(4)) ? "apk" : "apk-permission";
}

export async function installUpdate() {
  if (updater.busy) return;
  Object.assign(updater, { busy: true, pct: 0, done: "", error: "" });
  const off = await listen<{ key: string; pct: number }>("release-download", ({ payload }) => {
    if (payload.key === "update") updater.pct = payload.pct;
  });
  try {
    updater.done = openApk(await invoke<string>("update_install"));
  } catch (e) {
    updater.error = String(e);
  } finally {
    off();
    updater.busy = false;
  }
}
