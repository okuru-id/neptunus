import { invoke } from "@tauri-apps/api/core";
import { reactive } from "vue";

// Cover data URLs by track path (192px thumbnails for lists; the playing song gets a 512px one from App.vue); null means the file has no cover.
export const covers = reactive<Record<string, string | null>>({});

// Loads go through a queue, 3 at a time, so a fast scroll doesn't fire hundreds of requests at once.
// Newest first: the covers that just scrolled into view matter more than the ones scrolled past.
const MAX_ACTIVE = 3;
const waiting: string[] = [];
const queued = new Set<string>();
// Paths waiting in the queue or being fetched; the art shows a shimmer meanwhile.
export const pending = reactive<Record<string, true>>({});
let active = 0;

function pump() {
  while (active < MAX_ACTIVE && waiting.length) {
    const path = waiting.pop()!;
    queued.delete(path);
    active++;
    invoke<string | null>("get_cover", { path, thumb: true })
      .then((c) => (covers[path] = c))
      .catch(() => (covers[path] = null))
      .finally(() => {
        delete pending[path];
        active--;
        pump();
      });
  }
}

export function loadCover(path: string) {
  if (path in covers || queued.has(path)) return;
  queued.add(path);
  pending[path] = true;
  waiting.push(path);
  pump();
}
