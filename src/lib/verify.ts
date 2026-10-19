import { invoke } from "@tauri-apps/api/core";
import { ref } from "vue";

// SpotiFLAC gateway downloads may want a one-time browser check. Shared by the catalog and playlists,
// so a verification started in one place is the same one everywhere.
export const waiting = ref(false);
// the callback link pasted by hand when the browser cannot hand it back to the app
export const pasted = ref("");
export const verifyError = ref("");

async function verified(id: string) {
  if (!(await invoke<boolean>("ext_session_verify", { id }))) return false;
  verifyError.value = "";
  waiting.value = true;
  try {
    for (let i = 0; i < 300; i++) {
      await new Promise((r) => setTimeout(r, 1000));
      const text = pasted.value.trim() || null;
      pasted.value = "";
      if (await invoke<string | null>("ext_session_poll", { text })) return true;
      if (text) verifyError.value = "Tautan itu tidak cocok dengan verifikasi yang sedang berjalan.";
    }
    verifyError.value = "Verifikasi tidak selesai.";
    return false;
  } finally {
    waiting.value = false;
  }
}

/** Runs a download with extension `id`; when its gateway asks for verification, verifies and tries once more. */
export async function withVerify<T>(id: string, run: () => Promise<T>, retrying?: () => void): Promise<T> {
  try {
    return await run();
  } catch (e) {
    if (!/verif/i.test(String(e)) || !(await verified(id))) throw e;
    retrying?.();
    return run();
  }
}
