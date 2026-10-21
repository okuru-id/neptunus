<script lang="ts">
import { reactive } from "vue";

const ROOT = "/storage/emulated/0/";
const state = reactive<{ dir: string | null; resolve: (dir: string | null) => void }>({ dir: null, resolve: () => {} });

/** Android has no system folder picker in Tauri: browse the phone's storage in-app. Resolves the chosen folder, or null. */
export function chooseFolder(): Promise<string | null> {
  state.resolve(null);
  return new Promise((resolve) => {
    state.dir = `${ROOT}Music/`;
    state.resolve = resolve;
  });
}
</script>

<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { ref, watch } from "vue";
import Icon from "./Icon.vue";

// Mount once (in App); open it with `chooseFolder()`. Lists folders through `local_list`, which READ_MEDIA_AUDIO lets read.
type Entry = { key: string; name: string; dir: boolean };
const dirs = ref<Entry[]>([]);
const error = ref("");

watch(
  () => state.dir,
  async (dir) => {
    if (!dir) return;
    error.value = "";
    try {
      dirs.value = (await invoke<Entry[]>("local_list", { prefix: dir })).filter((e) => e.dir);
    } catch (e) {
      dirs.value = [];
      error.value = String(e);
    }
  },
);

const label = (dir: string) => (dir === ROOT ? "Memori internal" : dir.slice(ROOT.length, -1).replaceAll("/", " › "));
const up = () => state.dir && (state.dir = state.dir.slice(0, state.dir.slice(0, -1).lastIndexOf("/") + 1));
function close(dir: string | null) {
  state.dir = null;
  state.resolve(dir && dir.slice(0, -1));
  state.resolve = () => {};
}
</script>

<template>
  <Transition name="sheet">
    <div v-if="state.dir" class="scrim" @pointerdown.self="close(null)">
      <div class="sheet" role="dialog" aria-modal="true" aria-label="Pilih folder musik">
        <header>
          <button class="icon-btn" :disabled="state.dir === ROOT" aria-label="Folder induk" @click="up"><Icon name="back" :size="22" /></button>
          <div class="where">
            <strong>Pilih folder musik</strong>
            <span>{{ label(state.dir) }}</span>
          </div>
          <button class="icon-btn" aria-label="Tutup" @click="close(null)"><Icon name="close" :size="22" /></button>
        </header>
        <ul>
          <li v-for="d in dirs" :key="d.key">
            <button @click="state.dir = d.key"><Icon name="folder" :size="22" /><span>{{ d.name }}</span><Icon name="chevron" :size="20" class="go" /></button>
          </li>
          <li v-if="!dirs.length" class="empty">{{ error || "Tidak ada subfolder." }}</li>
        </ul>
        <button class="pick" @click="close(state.dir)">Pilih folder ini</button>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.scrim { position: fixed; inset: 0; z-index: 50; display: flex; align-items: flex-end; background: rgb(0 0 0 / 0.4); }
.sheet {
  width: 100%; max-height: 80vh; display: flex; flex-direction: column; gap: 8px;
  padding: 12px 16px calc(16px + env(safe-area-inset-bottom)); border-radius: 28px 28px 0 0;
  background: var(--surface-container-low); color: var(--on-surface);
}
header { flex: none; display: flex; align-items: center; gap: 8px; }
.where { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.where strong { font-size: 17px; font-weight: 700; }
.where span { font-size: 13px; color: var(--on-surface-variant); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.icon-btn { width: 44px; height: 44px; flex: none; display: grid; place-items: center; border-radius: 22px; color: var(--on-surface-variant); }
.icon-btn:disabled { opacity: 0.38; }
ul { list-style: none; margin: 0; padding: 0; overflow-y: auto; flex: 1; min-height: 0; border-radius: 20px; background: var(--surface-container); }
li + li { box-shadow: inset 0 1px 0 var(--outline-variant); }
li button { width: 100%; height: 52px; display: flex; align-items: center; gap: 14px; padding: 0 16px; text-align: left; font-size: 15px; font-weight: 500; }
li button > :first-child { color: var(--primary); }
li button span { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.go { color: var(--on-surface-variant); }
.empty { padding: 16px; font-size: 14px; color: var(--on-surface-variant); }
.pick { flex: none; height: 52px; border-radius: 26px; font-size: 15px; font-weight: 600; background: var(--primary); color: var(--on-primary); }
.sheet-enter-active, .sheet-leave-active { transition: opacity 0.25s var(--emph); }
.sheet-enter-active .sheet, .sheet-leave-active .sheet { transition: transform 0.3s var(--emph); }
.sheet-enter-from, .sheet-leave-to { opacity: 0; }
.sheet-enter-from .sheet, .sheet-leave-to .sheet { transform: translateY(100%); }
</style>
