import { invoke } from "@tauri-apps/api/core";
import { reactive } from "vue";

// Cover data URLs by track path; null means the file has no cover.
export const covers = reactive<Record<string, string | null>>({});

export async function loadCover(path: string) {
  if (!(path in covers)) covers[path] = await invoke<string | null>("get_cover", { path });
}
