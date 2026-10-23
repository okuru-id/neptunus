<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import Icon from "./Icon.vue";
import VideoPlayer from "./VideoPlayer.vue";
import VideoCatalogs from "./VideoCatalogs.vue";
import type { ExtInfo } from "./ext";
import type { SubtitleQuery } from "./videoext";

type Video = { path: string; title: string; size?: number; sub?: SubtitleQuery };
type Tab = "local" | "cloud" | "ext";

const props = defineProps<{ query: string; exts: ExtInfo[]; home?: number }>();
const emit = defineEmits<{ play: []; count: [n: number]; exts: [list: ExtInfo[]]; detail: [on: boolean]; watching: [on: boolean] }>();

const TAB_KEY = "neptunus.videoTab";
const TABS: { id: Tab; label: string; icon: "folder" | "cloud" | "extension" }[] = [
  { id: "ext", label: "Ekstensi", icon: "extension" },
  { id: "local", label: "Lokal", icon: "folder" },
  { id: "cloud", label: "Cloud S3", icon: "cloud" },
];
const stored = (() => { try { return localStorage.getItem(TAB_KEY); } catch { return null; } })();
const tab = ref<Tab>(TABS.some((t) => t.id === stored) ? (stored as Tab) : "ext");
watch(tab, (t) => { try { localStorage.setItem(TAB_KEY, t); } catch {} });
// the Ekstensi tab clicked again, or the Video menu: the catalogs leave an open title
const extHome = ref(0);
const inDetail = ref(false);
const reload = ref(0);
const refreshing = ref(false);
watch(() => props.home, () => { tab.value = "ext"; extHome.value++; });
function pickTab(t: Tab) {
  if (t === tab.value && t === "ext") extHome.value++;
  tab.value = t;
}

const videos = ref<Video[] | null>(null);
const error = ref("");
const open = ref<Video | null>(null);

const inTab = computed(() => (videos.value ?? []).filter((v) => v.path.startsWith("s3://") === (tab.value === "cloud")));
const shown = computed(() => {
  const q = props.query.trim().toLowerCase();
  return inTab.value.filter((v) => !q || v.title.toLowerCase().includes(q));
});
watch(inTab, (l) => emit("count", l.length), { immediate: true });
watch(() => tab.value === "ext" && inDetail.value, (on) => emit("detail", on), { immediate: true });
watch(() => !!open.value, (on) => emit("watching", on));
onUnmounted(() => emit("watching", false));
const size = (b?: number) =>
  b == null ? "" : b >= 1 << 30 ? `${(b / (1 << 30)).toFixed(1)} GB` : `${Math.round(b / (1 << 20))} MB`;

function play(v: Video) {
  emit("play"); // the music pauses while a video plays
  open.value = v;
}

onMounted(async () => {
  try {
    videos.value = await invoke<Video[]>("video_list");
  } catch (e) {
    error.value = String(e);
    videos.value = [];
  }
});
</script>

<template>
  <div class="tabbar">
    <div class="tabs" role="tablist" aria-label="Sumber video">
      <button v-for="t in TABS" :key="t.id" role="tab" :aria-selected="tab === t.id" :class="{ on: tab === t.id }" @click="pickTab(t.id)">
        <Icon :name="t.icon" :size="20" />{{ t.label }}
      </button>
    </div>
    <button v-if="tab === 'ext' && inDetail" class="back" title="Kembali ke katalog" @click="extHome++"><Icon name="back" :size="22" /></button>
    <button v-if="tab === 'ext' && !inDetail" class="reload" :disabled="refreshing" title="Ambil ulang daftar film dari semua ekstensi" @click="reload++">
      <Icon name="refresh" :size="20" :class="{ spinning: refreshing }" /><span class="lbl">{{ refreshing ? "Memuat ulang…" : "Muat ulang katalog" }}</span>
    </button>
  </div>

  <VideoCatalogs v-if="tab === 'ext'" :home="extHome" :exts="exts" @detail="inDetail = $event" :reload="reload" @refreshing="refreshing = $event" :query="query" @play="play" @changed="emit('exts', $event)" />

  <template v-else>
    <p v-if="videos === null" class="none">Mencari video…</p>
    <p v-else-if="error" class="none">{{ error }}</p>
    <p v-else-if="!inTab.length" class="none">
      <template v-if="tab === 'local'">Belum ada video. Video (MP4, MKV, WebM, MOV) di folder musik akan muncul di sini.</template>
      <template v-else>Belum ada video di bucket S3, atau S3 belum dihubungkan (Pengaturan › Cloud).</template>
    </p>
    <div v-else class="vgrid">
      <button v-for="v in shown" :key="v.path" class="vtile" @click="play(v)">
        <div class="thumb"><Icon :name="tab === 'cloud' ? 'cloud' : 'movie'" :size="40" /></div>
        <strong>{{ v.title }}</strong>
        <span v-if="v.size">{{ size(v.size) }}</span>
      </button>
      <p v-if="!shown.length" class="none">Tidak ada yang cocok dengan “{{ query }}”.</p>
    </div>
  </template>

  <VideoPlayer v-if="open" :key="open.path" :path="open.path" :title="open.title" :sub="open.sub" @close="open = null" />
</template>

<style scoped>
.none { color: var(--on-surface-variant); font-size: 15px; padding: 24px 12px; }
/* stays under the app header while the page scrolls; spans main's padding so nothing shows beside it */
.tabbar {
  position: sticky; top: var(--top-h, 0px); z-index: 3; display: flex; align-items: center; gap: 8px;
  margin: -12px calc(-1 * var(--pad-r, 8px)) 24px calc(-1 * var(--pad-l, 16px));
  padding: 0 var(--pad-r, 8px) 12px var(--pad-l, 16px); background: var(--surface);
}
.back {
  flex: none; width: 52px; height: 52px; border-radius: 26px; display: grid; place-items: center;
  background: var(--surface-container-low); color: var(--on-surface); transition: background 0.2s, border-radius 0.3s var(--spring);
}
.back:hover { background: var(--surface-container-highest); }
.back:active { border-radius: 14px; }
.reload {
  flex: none; margin-left: auto; height: 44px; padding: 0 18px 0 14px; border-radius: 22px; display: inline-flex; align-items: center; gap: 8px;
  font-size: 14px; font-weight: 600; background: var(--secondary-container); color: var(--on-secondary-container); transition: background 0.2s, opacity 0.2s;
}
.reload:hover:not(:disabled) { background: var(--surface-container-highest); }
.reload:disabled { opacity: 0.6; }
.spinning { animation: spin 0.9s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (max-width: 600px) { .reload { width: 44px; padding: 0; justify-content: center; } .reload .lbl { display: none; } }
.tabs { display: flex; gap: 4px; padding: 4px; border-radius: 28px; background: var(--surface-container-low); width: fit-content; max-width: 100%; overflow-x: auto; scrollbar-width: none; }
.tabs button {
  flex: none; height: 44px; padding: 0 18px 0 14px; border-radius: 22px; display: inline-flex; align-items: center; gap: 8px;
  font-size: 15px; font-weight: 600; color: var(--on-surface-variant); transition: background 0.2s, color 0.2s;
}
.tabs button:hover { background: var(--surface-container-highest); }
.tabs button.on { background: var(--secondary-container); color: var(--on-secondary-container); }
.vgrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 24px 20px; }
.vtile { display: flex; flex-direction: column; gap: 4px; text-align: left; min-width: 0; }
.thumb {
  position: relative; aspect-ratio: 16 / 9; margin-bottom: 8px; border-radius: 20px; display: grid; place-items: center;
  background: var(--secondary-container); color: var(--on-secondary-container); overflow: hidden;
  transition: border-radius 0.4s var(--spring), transform 0.45s var(--spring);
}
.vtile:hover .thumb { border-radius: 32px; transform: scale(1.03); }
.vtile strong { font-size: 15px; font-weight: 650; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.vtile span { font-size: 13px; color: var(--on-surface-variant); }
@media (max-width: 600px) {
  .tabs button { padding: 0 14px 0 12px; font-size: 14px; }
  .vgrid { grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 16px 12px; }
}
</style>
