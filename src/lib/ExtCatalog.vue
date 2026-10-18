<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import Icon from "./Icon.vue";
import { extName, has, unsupported, type ExtInfo } from "./ext";

// Katalog from metadata-provider extensions: home feed, search, album/playlist/artist pages,
// and downloading tracks into the library with a download-provider extension.
type Item = {
  id: string; name: string; artists?: string; album_name?: string; cover_url?: string; images?: string;
  image_url?: string; duration_ms?: number; item_type?: string; type?: string; provider_id?: string;
  release_date?: string; isrc?: string; description?: string; owner?: string;
};
type Section = { title: string; items: Item[] };
type Page = { kind: string; title: string; sub: string; cover?: string; tracks: Item[]; albums: Item[] };
type Track = { path: string; title: string; artist: string; album: string; duration_secs: number };

// `query` is the page's search box: typing searches the selected source (debounced)
const props = defineProps<{ exts: ExtInfo[]; query: string }>();
const emit = defineEmits<{ downloaded: [Track] }>();

const SRC_KEY = "neptunus.extSource";
const sources = computed(() => props.exts.filter((e) => e.enabled && has(e, "metadata_provider")));
const src = ref(localStorage.getItem(SRC_KEY) ?? "");
const cur = computed(() => sources.value.find((e) => e.id === src.value) ?? sources.value[0]);
// installed download extensions that need SpotiFLAC's signed-session server
const blocked = computed(() => props.exts.filter((e) => e.enabled && has(e, "download_provider") && unsupported(e).length).map(extName));
// downloads go through the source itself when it can, else the first usable download provider
const downloader = computed(() => {
  const ok = (e: ExtInfo) => e.enabled && has(e, "download_provider") && !unsupported(e).length;
  return cur.value && ok(cur.value) ? cur.value : props.exts.find(ok);
});

const loading = ref(false);
const error = ref("");
const home = ref<Section[]>([]);
const results = ref<Item[] | null>(null);
const pages = ref<Page[]>([]);
const page = computed(() => pages.value[pages.value.length - 1]);
const homeCache = new Map<string, { at: number; sections: Section[] }>();

const kind = (i: Item) => (i.item_type || i.type || "track").toLowerCase();
const cover = (i: Item) => i.cover_url || i.images || i.image_url || "";
const sub = (i: Item) => {
  const k = kind(i);
  if (k === "artist") return "Artis";
  if (k === "playlist") return i.owner || i.artists || i.description || "Playlist";
  if (k === "album") return [i.artists, i.release_date?.slice(0, 4)].filter(Boolean).join(" · ");
  return i.artists || "";
};
const dur = (ms?: number) => (ms ? `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}` : "");

async function call<T>(method: string, args: unknown[]): Promise<T> {
  return invoke<T>("ext_call", { id: cur.value!.id, method, args });
}
async function run(f: () => Promise<void>) {
  loading.value = true;
  error.value = "";
  try {
    await f();
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function loadHome() {
  home.value = [];
  const e = cur.value;
  if (!e?.capabilities?.homeFeed) return;
  const hit = homeCache.get(e.id);
  if (hit && Date.now() - hit.at < 5 * 60_000) return void (home.value = hit.sections);
  await run(async () => {
    const res = await call<{ sections?: Section[]; error?: string }>("getHomeFeed", []);
    if (res?.error) throw res.error;
    const sections = (res?.sections ?? []).filter((s) => s.items?.length);
    homeCache.set(e.id, { at: Date.now(), sections });
    if (cur.value?.id === e.id) home.value = sections;
  });
}

async function search() {
  const q = props.query.trim();
  pages.value = [];
  if (!q) return void (results.value = null);
  await run(async () => {
    let res: Item[] | null;
    try {
      res = await call<Item[]>("customSearch", [q, {}]);
    } catch (e) {
      if (!String(e).includes("not defined")) throw e;
      res = await call<Item[]>("searchTracks", [q, 30]);
    }
    // some providers wrap results ({ tracks: [...] })
    const list = Array.isArray(res) ? res : ((res as unknown as { tracks?: Item[] })?.tracks ?? []);
    results.value = list.filter((i) => i && i.id);
  });
}

async function openItem(i: Item) {
  const k = kind(i);
  if (k === "track") return download(i);
  const method = k === "artist" ? "getArtist" : k === "playlist" ? "getPlaylist" : "getAlbum";
  await run(async () => {
    const r = await call<Item & { tracks?: Item[]; albums?: Item[]; top_tracks?: Item[]; error?: string }>(method, [i.id]);
    if (!r || r.error) throw r?.error || "Tidak ditemukan";
    pages.value.push({
      kind: k,
      title: r.name || i.name,
      sub: k === "artist" ? "Artis" : sub({ ...i, ...r, item_type: k }),
      cover: cover(r) || cover(i),
      tracks: (r.tracks ?? r.top_tracks ?? []).map((t) => ({ ...t, cover_url: cover(t) || cover(r) || cover(i), album_name: t.album_name || (k === "album" ? r.name : undefined) })),
      albums: (r.albums ?? []).map((a) => ({ ...a, item_type: "album" })),
    });
  });
}

// ---- downloads
const dl = reactive<Record<string, number | "done" | string>>({});
const done = new Map<string, Track>();
const keyOf = (t: Item) => `${t.provider_id || cur.value?.id}:${t.id}`;
async function download(t: Item) {
  const key = keyOf(t);
  if (done.has(key)) return emit("downloaded", done.get(key)!);
  if (!downloader.value || typeof dl[key] === "number") return;
  dl[key] = 0;
  try {
    const track = await invoke<Track>("ext_download", {
      id: downloader.value.id, key, quality: null,
      track: { ...t, provider_id: t.provider_id || cur.value?.id },
    });
    dl[key] = "done";
    done.set(key, track);
    emit("downloaded", track);
  } catch (e) {
    dl[key] = String(e);
  }
}
let unlisten: UnlistenFn | undefined;
onMounted(async () => {
  unlisten = await listen<{ key: string; pct: number }>("ext-download", (e) => {
    if (typeof dl[e.payload.key] === "number") dl[e.payload.key] = e.payload.pct;
  });
});
onUnmounted(() => unlisten?.());

watch(() => cur.value?.id, (id) => {
  if (!id) return;
  localStorage.setItem(SRC_KEY, id);
  pages.value = [];
  if (props.query.trim()) search();
  else loadHome();
}, { immediate: true });
let timer: ReturnType<typeof setTimeout> | undefined;
watch(() => props.query, () => {
  clearTimeout(timer);
  timer = setTimeout(search, 600);
});

const tracks = computed(() => results.value?.filter((i) => kind(i) === "track") ?? []);
const others = computed(() => results.value?.filter((i) => kind(i) !== "track") ?? []);
</script>

<template>
  <section v-if="sources.length" class="xc">
    <div class="bar">
      <div class="srcs" role="tablist">
        <button v-for="e in sources" :key="e.id" class="src" :class="{ sel: cur?.id === e.id }" role="tab" :aria-selected="cur?.id === e.id" @click="src = e.id">
          <img v-if="e.iconData" :src="e.iconData" alt="" /><Icon v-else name="extension" :size="18" />{{ extName(e) }}
        </button>
      </div>
    </div>

    <p v-if="error" class="note err">{{ error }}</p>
    <p v-if="!downloader && cur" class="note">
      <template v-if="blocked.length">{{ blocked.join(", ") }} hanya bisa mengunduh lewat server SpotiFLAC, jadi tidak bisa dipakai di sini.</template>
      Pasang ekstensi unduhan seperti YouTube Music atau JioSaavn untuk menyimpan lagu dari katalog ke library.
    </p>

    <!-- album / playlist / artist page -->
    <div v-if="page" class="page">
      <div class="page-head">
        <button class="icon-btn tonal" title="Kembali" @click="pages.pop()"><Icon name="back" /></button>
        <img v-if="page.cover" :src="page.cover" class="page-art" :class="{ round: page.kind === 'artist' }" alt="" />
        <div><h2>{{ page.title }}</h2><span>{{ page.sub }}</span></div>
      </div>
      <ul v-if="page.tracks.length" class="tracks">
        <li v-for="(t, i) in page.tracks" :key="t.id + i">
          <img v-if="cover(t)" :src="cover(t)" alt="" loading="lazy" />
          <span class="t-meta"><strong>{{ t.name }}</strong><span>{{ t.artists }}</span></span>
          <span class="t-dur">{{ dur(t.duration_ms) }}</span>
          <button v-if="downloader" class="icon-btn dl" :class="{ ok: dl[keyOf(t)] === 'done' }" @click="download(t)"
            :title="dl[keyOf(t)] === 'done' ? 'Putar' : typeof dl[keyOf(t)] === 'string' ? `Gagal: ${dl[keyOf(t)]}` : 'Unduh ke library'">
            <span v-if="typeof dl[keyOf(t)] === 'number'" class="ring" :style="{ '--p': dl[keyOf(t)] }"></span>
            <Icon v-else :name="dl[keyOf(t)] === 'done' ? 'play' : typeof dl[keyOf(t)] === 'string' ? 'refresh' : 'download'" />
          </button>
        </li>
      </ul>
      <div v-if="page.albums.length" class="row">
        <button v-for="a in page.albums" :key="a.id" class="tile" @click="openItem(a)">
          <img :src="cover(a)" alt="" loading="lazy" /><strong>{{ a.name }}</strong><span>{{ sub(a) }}</span>
        </button>
      </div>
    </div>

    <!-- search results -->
    <template v-else-if="results">
      <ul v-if="tracks.length" class="tracks">
        <li v-for="(t, i) in tracks.slice(0, 20)" :key="t.id + i">
          <img v-if="cover(t)" :src="cover(t)" alt="" loading="lazy" />
          <span class="t-meta"><strong>{{ t.name }}</strong><span>{{ [t.artists, t.album_name].filter(Boolean).join(" · ") }}</span></span>
          <span class="t-dur">{{ dur(t.duration_ms) }}</span>
          <button v-if="downloader" class="icon-btn dl" :class="{ ok: dl[keyOf(t)] === 'done' }" @click="download(t)"
            :title="dl[keyOf(t)] === 'done' ? 'Putar' : typeof dl[keyOf(t)] === 'string' ? `Gagal: ${dl[keyOf(t)]}` : 'Unduh ke library'">
            <span v-if="typeof dl[keyOf(t)] === 'number'" class="ring" :style="{ '--p': dl[keyOf(t)] }"></span>
            <Icon v-else :name="dl[keyOf(t)] === 'done' ? 'play' : typeof dl[keyOf(t)] === 'string' ? 'refresh' : 'download'" />
          </button>
        </li>
      </ul>
      <div v-if="others.length" class="row">
        <button v-for="a in others" :key="a.id" class="tile" :class="{ artist: kind(a) === 'artist' }" @click="openItem(a)">
          <img :src="cover(a)" alt="" loading="lazy" /><strong>{{ a.name }}</strong><span>{{ sub(a) }}</span>
        </button>
      </div>
      <p v-if="!loading && !results.length" class="note">Tidak ada hasil di {{ cur ? extName(cur) : "" }} untuk “{{ props.query }}”.</p>
    </template>

    <!-- home feed -->
    <template v-else>
      <section v-for="s in home" :key="s.title" class="shelf">
        <h2>{{ s.title }}</h2>
        <div class="row">
          <button v-for="(a, i) in s.items" :key="a.id + i" class="tile" :class="{ artist: kind(a) === 'artist' }" @click="openItem(a)"
            :title="kind(a) === 'track' && downloader ? 'Unduh ke library' : undefined">
            <span class="art">
              <img :src="cover(a)" alt="" loading="lazy" />
              <span v-if="kind(a) === 'track' && dl[keyOf(a)] !== undefined" class="badge">
                <span v-if="typeof dl[keyOf(a)] === 'number'" class="ring" :style="{ '--p': dl[keyOf(a)] }"></span>
                <Icon v-else :name="dl[keyOf(a)] === 'done' ? 'play' : 'refresh'" :size="20" />
              </span>
            </span>
            <strong>{{ a.name }}</strong><span>{{ sub(a) }}</span>
          </button>
        </div>
      </section>
    </template>
    <p v-if="loading" class="loading"><span class="spin"></span>Memuat dari {{ cur ? extName(cur) : "" }}…</p>
  </section>
</template>

<style scoped>
.xc { display: flex; flex-direction: column; gap: 24px; }
.bar { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
.srcs { display: flex; gap: 8px; overflow-x: auto; scrollbar-width: none; flex: 1 1 auto; min-width: 0; }
.src {
  display: inline-flex; align-items: center; gap: 8px; height: 40px; padding: 0 16px 0 10px; border-radius: 12px; flex: none;
  font-size: 14px; font-weight: 600; color: var(--on-surface-variant); box-shadow: inset 0 0 0 1px var(--outline-variant); transition: background 0.2s;
}
.src img { width: 22px; height: 22px; border-radius: 6px; object-fit: cover; }
.src:hover { background: var(--surface-container-high); }
.src.sel { background: var(--secondary-container); color: var(--on-secondary-container); box-shadow: none; }
.icon-btn {
  width: 40px; height: 40px; border-radius: 20px; display: inline-grid; place-items: center; flex: none; position: relative;
  color: var(--on-surface-variant); transition: background 0.2s, border-radius 0.3s var(--spring);
}
.icon-btn:hover { background: var(--surface-container-highest); }
.icon-btn.tonal { background: var(--secondary-container); color: var(--on-secondary-container); }
.note { margin: 0; padding: 12px 16px; border-radius: 12px; background: var(--secondary-container); color: var(--on-secondary-container); font-size: 14px; }
.note.err { background: #8c1d18; color: #ffdad6; overflow-wrap: anywhere; }
.loading { display: flex; align-items: center; gap: 10px; margin: 0; color: var(--on-surface-variant); font-size: 14px; }
.spin { width: 18px; height: 18px; border-radius: 50%; border: 2px solid var(--primary); border-right-color: transparent; animation: spin 0.8s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }

.shelf h2, .page-head h2 { margin: 0 0 16px; font-size: 24px; font-weight: 750; letter-spacing: -0.01em; font-variation-settings: "wdth" 112; }
.row {
  display: grid; grid-auto-flow: column; grid-auto-columns: 168px; gap: 20px;
  overflow-x: auto; padding: 4px 4px 12px; margin: -4px -4px 0; scrollbar-width: thin; scrollbar-color: var(--outline-variant) transparent;
}
.tile { display: flex; flex-direction: column; gap: 4px; text-align: left; min-width: 0; }
.tile .art { position: relative; display: block; }
.tile img { width: 100%; aspect-ratio: 1; object-fit: cover; border-radius: 20px; margin-bottom: 10px; background: var(--surface-container-high); transition: transform 0.45s var(--spring), border-radius 0.45s var(--spring); }
.tile:hover img { transform: scale(1.04); border-radius: 32px; }
.tile.artist { align-items: center; text-align: center; }
.tile.artist img { border-radius: 50%; }
.tile strong { max-width: 100%; font-size: 16px; font-weight: 650; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.tile > span:not(.art) { max-width: 100%; font-size: 14px; color: var(--on-surface-variant); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.badge { position: absolute; right: 8px; bottom: 18px; width: 40px; height: 40px; border-radius: 20px; display: grid; place-items: center; background: var(--primary); color: var(--on-primary); }

.tracks { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
.tracks li { display: flex; align-items: center; gap: 16px; padding: 8px 12px; border-radius: 16px; transition: background 0.2s; }
.tracks li:hover { background: var(--surface-container); }
.tracks img { width: 48px; height: 48px; border-radius: 12px; object-fit: cover; flex: none; }
.t-meta { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.t-meta strong { font-size: 15px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.t-meta span { font-size: 13px; color: var(--on-surface-variant); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.t-dur { font-size: 13px; color: var(--on-surface-variant); font-variant-numeric: tabular-nums; }
.dl.ok { color: var(--primary); }
/* download progress ring (--p: 0-100) */
.ring { width: 22px; height: 22px; border-radius: 50%; background: conic-gradient(var(--primary) calc(var(--p) * 1%), color-mix(in srgb, var(--primary) 20%, transparent) 0);
  mask: radial-gradient(circle, transparent 6px, #000 7px); }
.badge .ring { background: conic-gradient(var(--on-primary) calc(var(--p) * 1%), color-mix(in srgb, var(--on-primary) 30%, transparent) 0); }

.page { display: flex; flex-direction: column; gap: 20px; }
.page-head { display: flex; align-items: center; gap: 20px; }
.page-head h2 { margin: 0 0 4px; }
.page-head span { color: var(--on-surface-variant); }
.page-art { width: 120px; height: 120px; border-radius: 24px; object-fit: cover; flex: none; }
.page-art.round { border-radius: 50%; }
@media (max-width: 600px) { .page-art { width: 72px; height: 72px; } }
</style>
