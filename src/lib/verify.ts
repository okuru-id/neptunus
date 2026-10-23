import { invoke } from "@tauri-apps/api/core";
import { ref } from "vue";

// SpotiFLAC gateway downloads may want a one-time browser check. It is done from Settings › Verifikasi;
// a download that runs into it only says so (`asked`), so a page never opens in the middle of one.
export const waiting = ref<string | null>(null); // extension whose verification page is open
// the callback link pasted by hand when the browser cannot hand it back to the app
export const pasted = ref("");
export const verifyError = ref("");
/** Counts verifications finished outside `verify` (phones: when the app loads again), so the settings refresh. */
export const verified = ref(0);
/** The last download that needed a verification: its extension, and a counter so each one is seen. */
export const asked = ref<{ id: string; n: number } | null>(null);

export const needsVerify = (e: unknown) => /verif/i.test(String(e));

/** Waits for the grant of the verification page just opened for `id`. */
async function grant(id: string) {
  verifyError.value = "";
  waiting.value = id;
  try {
    for (let i = 0; i < 300; i++) {
      await new Promise((r) => setTimeout(r, 1000));
      const text = pasted.value.trim() || null;
      pasted.value = "";
      try {
        if (await invoke<string | null>("ext_session_poll", { text })) return true;
      } catch (e) {
        // a pasted link from another verification: keep waiting for the right one
        if (!text) throw e;
        verifyError.value = "Tautan itu tidak cocok dengan verifikasi yang sedang berjalan.";
      }
    }
    verifyError.value = "Verifikasi tidak selesai.";
    return false;
  } finally {
    waiting.value = null;
  }
}

/** Verifies extension `id` with a fresh challenge. On phones the page replaces the app, which picks the
 *  grant up when it loads again; elsewhere this waits for it. */
export async function verify(id: string) {
  if (!(await invoke<boolean>("ext_session_start", { id }))) return true; // a session without a challenge
  return grant(id);
}

/** Runs a download with extension `id`, noting in `asked` when it failed for want of a verification. */
export async function withVerify<T>(id: string, run: () => Promise<T>): Promise<T> {
  try {
    return await run();
  } catch (e) {
    if (needsVerify(e)) asked.value = { id, n: (asked.value?.n ?? 0) + 1 };
    throw e;
  }
}
