<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { computed, onMounted, onUnmounted, ref, watch, watchEffect } from "vue";
import CoverArt from "./lib/CoverArt.vue";
import { covers } from "./lib/covers";
import Icon from "./lib/Icon.vue";
import TitleBar from "./lib/TitleBar.vue";
import type { IconName } from "./lib/icons";
import WavyProgress from "./lib/WavyProgress.vue";
import { applyTheme, DEFAULT_SEED, seedFromImage } from "./lib/theme";
import { ART_SHAPES, CLIP_SHAPES } from "./lib/shapes";

type Track = { path: string; title: string; artist: string; album: string; duration_secs: number };
type Playlist = { id: number; name: string };
type View = "songs" | "albums" | "artists" | "playlists" | "s3";

const NAV: { id: View; label: string; icon: IconName }[] = [
  { id: "songs", label: "Lagu", icon: "note" },
  { id: "albums", label: "Album", icon: "album" },
  { id: "artists", label: "Artis", icon: "artist" },
  { id: "playlists", label: "Playlist", icon: "playlist" },
  { id: "s3", label: "Cloud", icon: "cloud" },
];

const library = ref<Track[]>([]);
const playlists = ref<Playlist[]>([]);
const loading = ref(false);

const view = ref<View>("songs");
const query = ref("");
const group = ref<string | null>(null); // selected album/artist name
const plId = ref<number | null>(null);
const plTracks = ref<Track[]>([]);
const newPlaylist = ref("");

type S3Config = { endpoint: string; region: string; bucket: string; prefix: string; access_key: string; secret_key: string };
const s3 = ref<S3Config>({ endpoint: "", region: "us-east-1", bucket: "", prefix: "", access_key: "", secret_key: "" });
const s3Status = ref("");
const s3Busy = ref(false);

const queue = ref<Track[]>([]);
const current = ref(-1);
const paused = ref(false);
const position = ref(0);
const volume = ref(0.8);
const shuffle = ref(false);
const repeat = ref<"off" | "all" | "one">("off");
let seeking = false;
let preloaded = -1; // queue index already appended to the player for a gapless handoff

const showNow = ref(false);
const lyrics = ref<{ t: number | null; text: string }[]>([]);

const now = computed<Track | undefined>(() => queue.value[current.value]);
const playing = computed(() => !!now.value && !paused.value);
const defs = ref<SVGSVGElement>();
const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
watchEffect(() => (playing.value ? defs.value?.unpauseAnimations() : defs.value?.pauseAnimations()));
const lyricIdx = computed(() =>
  lyrics.value[0]?.t == null ? -1 : lyrics.value.findLastIndex((l) => (l.t ?? Infinity) <= position.value + 0.2),
);
const q = computed(() => query.value.trim().toLowerCase());
const match = (t: Track) =>
  !q.value || [t.title, t.artist, t.album].some((s) => s.toLowerCase().includes(q.value));

const unknown = (s: string) => s || "Tidak diketahui";
const groupBy = (key: "album" | "artist") => {
  const m = new Map<string, Track[]>();
  for (const t of library.value.filter(match)) {
    const k = unknown(t[key]);
    m.set(k, [...(m.get(k) ?? []), t]);
  }
  return [...m.entries()].sort(([a], [b]) => a.localeCompare(b));
};
const albums = computed(() => groupBy("album"));
const artists = computed(() => groupBy("artist"));

// tracks shown in the main list, which also becomes the play queue
const shown = computed(() => {
  if (view.value === "songs") return library.value.filter(match);
  if (view.value === "playlists") return plTracks.value.filter(match);
  const g = (view.value === "albums" ? albums.value : artists.value).find(([k]) => k === group.value);
  return g ? g[1] : [];
});
const showingList = computed(() => view.value === "songs" || view.value === "playlists" || group.value !== null);
const currentPlaylist = computed(() => playlists.value.find((p) => p.id === plId.value));

const title = computed(() => {
  if (group.value !== null) return group.value;
  if (view.value === "playlists") return currentPlaylist.value?.name ?? "Playlist";
  if (view.value === "s3") return "Cloud";
  return NAV.find((n) => n.id === view.value)!.label;
});
const subtitle = computed(() => {
  if (view.value === "s3") return "Sinkronkan musik dari bucket S3";
  if (view.value === "albums" && group.value === null) return `${albums.value.length} album`;
  if (view.value === "artists" && group.value === null) return `${artists.value.length} artis`;
  const total = shown.value.reduce((s, t) => s + t.duration_secs, 0);
  return `${shown.value.length} lagu · ${Math.round(total / 60)} menit`;
});

async function refreshPlaylists() {
  playlists.value = await invoke<Playlist[]>("list_playlists");
}
async function openPlaylist(id: number) {
  plId.value = id;
  plTracks.value = await invoke<Track[]>("playlist_tracks", { id });
}
async function createPlaylist() {
  const name = newPlaylist.value.trim();
  if (!name) return;
  newPlaylist.value = "";
  const id = await invoke<number>("create_playlist", { name });
  await refreshPlaylists();
  await openPlaylist(id);
}
async function deletePlaylist(id: number) {
  await invoke("delete_playlist", { id });
  if (plId.value === id) { plId.value = null; plTracks.value = []; }
  await refreshPlaylists();
}
// In-page menu rather than a native <select>: WebKitGTK shows that as a GTK popup whose pointer grab can
// get stuck under XWayland, leaving the whole window unclickable.
const addMenu = ref<string | null>(null);
async function addTo(id: number, path: string) {
  addMenu.value = null;
  await invoke("add_to_playlist", { id, path });
  if (plId.value === id) await openPlaylist(plId.value);
}
function closeMenu(e: PointerEvent) {
  if (!(e.target as HTMLElement).closest(".row-menu")) addMenu.value = null;
}
async function removeFrom(path: string) {
  if (plId.value === null) return;
  await invoke("remove_from_playlist", { id: plId.value, path });
  await openPlaylist(plId.value);
}

function setView(v: View) {
  view.value = v;
  group.value = null;
}

async function pickFolder() {
  const dir = await open({ directory: true });
  if (!dir) return;
  loading.value = true;
  try {
    library.value = await invoke<Track[]>("scan_folder", { path: dir });
    setView("songs");
  } finally {
    loading.value = false;
  }
}

async function s3Save() {
  await invoke("s3_save_config", { config: s3.value });
  s3Status.value = "Tersimpan.";
}
async function s3Sync() {
  s3Busy.value = true;
  s3Status.value = "Menghubungkan…";
  try {
    await s3Save();
    library.value = await invoke<Track[]>("s3_sync");
    s3Status.value = "Selesai. Lagu sudah masuk ke library.";
  } catch (e) {
    s3Status.value = `Gagal: ${e}`;
  } finally {
    s3Busy.value = false;
  }
}

async function playAt(list: Track[], i: number) {
  if (!list[i]) return;
  queue.value = list;
  current.value = i;
  preloaded = -1;
  position.value = 0;
  paused.value = false;
  await invoke("play", { path: list[i].path, volume: volume.value });
}

function nextIndex(auto: boolean): number | null {
  if (!queue.value.length) return null;
  if (auto && repeat.value === "one") return current.value;
  if (shuffle.value) return Math.floor(Math.random() * queue.value.length);
  if (current.value + 1 < queue.value.length) return current.value + 1;
  return repeat.value === "all" ? 0 : null;
}

function next(auto = false) {
  const i = nextIndex(auto);
  if (i === null) current.value = -1;
  else playAt(queue.value, i);
}

function prev() {
  if (position.value > 3 || current.value <= 0) return playAt(queue.value, Math.max(current.value, 0));
  playAt(queue.value, current.value - 1);
}

async function togglePause() {
  if (current.value < 0) return playAt(shown.value, 0);
  paused.value = !paused.value;
  await invoke("set_paused", { paused: paused.value });
}

function playShuffled() {
  if (!shown.value.length) return;
  shuffle.value = true;
  playAt(shown.value, Math.floor(Math.random() * shown.value.length));
}

const cycleRepeat = () => (repeat.value = repeat.value === "off" ? "all" : repeat.value === "all" ? "one" : "off");

async function poll() {
  if (current.value < 0) return;
  const st = await invoke<{ position_secs: number; paused: boolean; finished: boolean; queued: number } | null>("status");
  if (!st) return;
  if (st.finished) {
    if (preloaded >= 0) current.value = preloaded;
    preloaded = -1;
    return next(true);
  }
  if (preloaded >= 0 && st.queued < 2) {
    // the preloaded track took over seamlessly
    current.value = preloaded;
    preloaded = -1;
  }
  if (!seeking) position.value = st.position_secs;
  if (preloaded < 0 && st.queued === 1 && now.value && now.value.duration_secs - st.position_secs < 10) {
    const i = nextIndex(true);
    if (i !== null) {
      preloaded = i;
      invoke("queue_next", { path: queue.value[i].path }).catch(() => (preloaded = -1));
    }
  }
}

function parseLyrics(raw: string | null) {
  if (!raw) return [];
  const out: { t: number | null; text: string }[] = [];
  for (const line of raw.split(/\r?\n/)) {
    const stamps = [...line.matchAll(/\[(\d+):(\d+(?:\.\d+)?)\]/g)];
    const text = line.replace(/\[[^\]]*\]/g, "").trim();
    if (stamps.length) for (const m of stamps) out.push({ t: +m[1] * 60 + +m[2], text });
    else if (!/^\[\w+:/.test(line)) out.push({ t: null, text: line.trim() });
  }
  return out.some((l) => l.t != null)
    ? out.filter((l) => l.t != null).sort((a, b) => a.t! - b.t!)
    : out;
}

// lyrics + dynamic colour follow the playing track
watch(() => now.value?.path, (path) => {
  lyrics.value = [];
  if (!path) return;
  const n = now.value!;
  invoke<string | null>("get_lyrics", { path, title: n.title, artist: n.artist, album: n.album, duration: n.duration_secs }).then((r) => {
    if (now.value?.path === path) lyrics.value = parseLyrics(r);
  });
  invoke<string | null>("get_cover", { path }).then(async (c) => {
    covers[path] = c;
    const seed = c ? await seedFromImage(c).catch(() => DEFAULT_SEED) : DEFAULT_SEED;
    if (now.value?.path === path) applyTheme(seed);
  });
});

watch([lyricIdx, showNow], () => {
  if (lyricIdx.value >= 0 && showNow.value)
    document.querySelector(".lyrics .cur")?.scrollIntoView({ block: "center", behavior: "smooth" });
}, { flush: "post" });

// OS media keys / MPRIS / SMTC
watch(now, (n) => {
  if (n)
    invoke("media_metadata", {
      path: n.path, title: n.title, artist: n.artist, album: n.album, durationSecs: n.duration_secs,
    });
});
watch([current, paused], () => {
  invoke("media_playback", { playing: current.value < 0 ? null : !paused.value, position: position.value });
}, { immediate: true });

// Mirror now-playing state to the tray mini player window (it sends commands back as media-key/media-seek).
const npRequests = ref(0);
watchEffect(() => {
  void npRequests.value;
  const n = now.value;
  emit("np-state", n ? {
    title: n.title, artist: unknown(n.artist), album: unknown(n.album),
    cover: covers[n.path] ?? null, paused: paused.value, position: position.value, duration: n.duration_secs,
  } : null);
});

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape" && addMenu.value) return void (addMenu.value = null);
  const el = e.target as HTMLElement;
  if (el.closest("input, select, textarea, button")) return;
  if (e.code === "Space") { e.preventDefault(); togglePause(); }
  else if (e.key === "Escape") showNow.value = false;
}

let pollId = 0;
const subs: Promise<UnlistenFn>[] = [];
onMounted(async () => {
  window.addEventListener("keydown", onKey);
  window.addEventListener("pointerdown", closeMenu);
  pollId = setInterval(poll, 500);
  subs.push(
    listen<[number, number]>("s3-progress", ({ payload: [i, n] }) => {
      s3Status.value = `Membaca ${i} dari ${n} lagu…`;
    }),
    listen<string>("media-key", ({ payload }) => {
      if (payload === "next") next();
      else if (payload === "prev") prev();
      else if (payload === "toggle" || (payload === "play") === paused.value) togglePause();
    }),
    listen<number>("media-seek", async ({ payload }) => {
      await invoke("seek", { secs: payload });
      position.value = payload;
    }),
    listen("np-request", () => npRequests.value++),
  );
  applyTheme(DEFAULT_SEED);
  library.value = await invoke<Track[]>("load_library");
  await refreshPlaylists();
  s3.value = await invoke<S3Config>("s3_get_config");
});
onUnmounted(() => {
  window.removeEventListener("keydown", onKey);
  window.removeEventListener("pointerdown", closeMenu);
  clearInterval(pollId);
  subs.forEach((p) => p.then((off) => off()));
});

function scrub(v: number) {
  seeking = true;
  position.value = v;
}
async function seekTo(v: number) {
  await invoke("seek", { secs: v });
  position.value = v;
  seeking = false;
}

const fmt = (s: number) => {
  s = Math.floor(s);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
};
const shapeOf = (i: number) => ART_SHAPES[i % ART_SHAPES.length];
const repeatLabel = computed(() => (repeat.value === "off" ? "mati" : repeat.value === "all" ? "semua" : "satu lagu"));
</script>

<template>
  <svg ref="defs" class="defs" aria-hidden="true">
    <defs>
      <clipPath v-for="(d, id) in CLIP_SHAPES" :key="id" :id="`shape-${id}`" clipPathUnits="objectBoundingBox"><path :d="d" /></clipPath>
      <!-- the shape itself spins: rotating a clipped element with CSS leaves stray edge lines in WebKitGTK -->
      <clipPath id="shape-now" clipPathUnits="objectBoundingBox">
        <path :d="CLIP_SHAPES.cookie12">
          <animateTransform v-if="!reduceMotion" attributeName="transform" type="rotate" from="0 0.5 0.5" to="360 0.5 0.5" dur="40s" repeatCount="indefinite" />
        </path>
      </clipPath>
    </defs>
  </svg>

  <TitleBar />

  <div class="app">
    <nav class="drawer" :class="{ 'has-player': !!now }" aria-label="Navigasi">
      <button class="fab" @click="pickFolder" :disabled="loading">
        <template v-if="loading"><span class="spinner" :style="{ clipPath: 'url(#shape-cookie9)' }"></span>Memindai…</template>
        <template v-else><Icon name="addFolder" />Tambah folder</template>
      </button>
      <div class="dest">
        <button v-for="n in NAV" :key="n.id" class="dest-item" :class="{ sel: view === n.id }" @click="setView(n.id)" :aria-current="view === n.id ? 'page' : undefined">
          <Icon :name="n.icon" />
          <span>{{ n.label }}</span>
          <span v-if="n.id === 'playlists' && playlists.length" class="badge">{{ playlists.length }}</span>
        </button>
      </div>
    </nav>

    <main :class="{ 'has-player': !!now }">
      <header class="top">
        <div class="heading">
          <button v-if="group !== null" class="icon-btn tonal" @click="group = null" title="Kembali"><Icon name="back" /></button>
          <div>
            <h1>{{ title }}</h1>
            <p v-if="library.length || view === 's3'" class="sub">{{ subtitle }}</p>
          </div>
        </div>
        <label v-if="library.length && view !== 's3'" class="search">
          <Icon name="search" />
          <input placeholder="Cari lagu, artis, album" v-model="query" />
          <button v-if="query" class="icon-btn" @click="query = ''" title="Hapus pencarian"><Icon name="close" :size="20" /></button>
        </label>
      </header>

      <p v-if="loading" class="scanning"><span class="spinner" :style="{ clipPath: 'url(#shape-cookie12)' }"></span>Memindai folder musik…</p>

      <section v-if="!library.length && view !== 's3'" class="empty">
        <div class="empty-mark" :style="{ clipPath: 'url(#shape-cookie12)' }"><Icon name="note" :size="72" /></div>
        <h2>Belum ada musik</h2>
        <p>Pilih folder berisi lagu di komputer ini, atau sinkronkan dari bucket S3.</p>
        <div class="btn-group">
          <button class="btn filled" @click="pickFolder"><Icon name="addFolder" />Pilih folder musik</button>
          <button class="btn tonal" @click="setView('s3')"><Icon name="cloud" />Hubungkan S3</button>
        </div>
      </section>
      <form v-else-if="view === 's3'" class="panel" @submit.prevent="s3Sync">
        <p class="hint">Lagu tidak diunduh: hanya metadata yang dibaca, lalu lagu di-stream langsung dari bucket saat diputar.</p>
        <div class="fields">
          <label class="field wide">Endpoint<input v-model="s3.endpoint" placeholder="Kosongkan untuk AWS, mis. https://minio.contoh.com" /></label>
          <label class="field">Region<input v-model="s3.region" /></label>
          <label class="field">Bucket<input v-model="s3.bucket" required /></label>
          <label class="field wide">Prefix<input v-model="s3.prefix" placeholder="Opsional, mis. music/" /></label>
          <label class="field">Access key<input v-model="s3.access_key" required autocomplete="off" /></label>
          <label class="field">Secret key<input type="password" v-model="s3.secret_key" required autocomplete="off" /></label>
        </div>
        <div class="btn-group">
          <button class="btn filled" :disabled="s3Busy"><Icon name="cloud" />{{ s3Busy ? "Menyinkronkan…" : "Simpan & sinkronkan" }}</button>
          <button type="button" class="btn tonal" @click="s3Save" :disabled="s3Busy">Simpan saja</button>
        </div>
        <p v-if="s3Status" class="status" :class="{ err: s3Status.startsWith('Gagal') }" role="status">{{ s3Status }}</p>
      </form>
      <template v-else>
        <div v-if="view === 'playlists'" class="chips" role="list">
          <span v-for="p in playlists" :key="p.id" class="chip" :class="{ sel: plId === p.id }" role="listitem">
            <button @click="openPlaylist(p.id)">{{ p.name }}</button>
            <button v-if="plId === p.id" class="chip-x" :title="`Hapus playlist ${p.name}`" @click="deletePlaylist(p.id)"><Icon name="close" :size="18" /></button>
          </span>
          <form class="chip new" @submit.prevent="createPlaylist">
            <Icon name="add" :size="18" />
            <input placeholder="Playlist baru" v-model="newPlaylist" aria-label="Nama playlist baru" />
          </form>
        </div>

        <div v-if="showingList && (view !== 'playlists' || plId !== null) && shown.length" class="btn-group actions">
          <button class="btn filled big" @click="playAt(shown, 0)"><Icon name="play" :size="28" />Putar</button>
          <button class="btn tonal big" @click="playShuffled"><Icon name="shuffle" />Acak</button>
        </div>

        <div v-if="(view === 'albums' || view === 'artists') && group === null" class="grid" :class="{ artists: view === 'artists' }">
          <button v-for="([name, list], i) in view === 'albums' ? albums : artists" :key="name" class="tile" @click="group = name">
            <CoverArt :path="list[0].path" :shape="view === 'artists' ? 'circle' : shapeOf(i)" />
            <strong>{{ name }}</strong>
            <span>{{ view === "albums" ? unknown(list[0].artist) : `${list.length} lagu` }}</span>
          </button>
          <p v-if="!(view === 'albums' ? albums : artists).length" class="none">Tidak ada yang cocok dengan “{{ query }}”.</p>
        </div>
        <ul v-else-if="showingList" class="songs">
          <li v-for="(t, i) in shown" :key="t.path" :class="{ active: now?.path === t.path }">
            <button class="row" @dblclick="playAt(shown, i)" @keydown.enter="playAt(shown, i)" title="Klik dua kali untuk memutar">
              <span class="art-wrap">
                <CoverArt :path="t.path" />
                <span v-if="now?.path === t.path" class="art-overlay">
                  <span class="eq" :class="{ still: paused }" aria-label="Sedang diputar"><i></i><i></i><i></i></span>
                </span>
              </span>
              <span class="meta">
                <strong>{{ t.title }}</strong>
                <span>{{ unknown(t.artist) }} · {{ unknown(t.album) }}</span>
              </span>
            </button>
            <button v-if="view === 'playlists'" class="icon-btn row-act" title="Hapus dari playlist" @click="removeFrom(t.path)"><Icon name="close" :size="20" /></button>
            <div v-else-if="playlists.length" class="row-menu" :class="{ open: addMenu === t.path }">
              <button class="icon-btn row-act" title="Tambah ke playlist" :aria-label="`Tambah ${t.title} ke playlist`"
                :aria-expanded="addMenu === t.path" @click="addMenu = addMenu === t.path ? null : t.path">
                <Icon name="playlistAdd" :size="20" />
              </button>
              <div v-if="addMenu === t.path" class="pl-menu" role="menu">
                <p>Tambah ke playlist</p>
                <button v-for="p in playlists" :key="p.id" role="menuitem" @click="addTo(p.id, t.path)">{{ p.name }}</button>
              </div>
            </div>
            <span class="dur">{{ fmt(t.duration_secs) }}</span>
          </li>
          <p v-if="!shown.length" class="none">
            <template v-if="view === 'playlists' && plId === null">
              {{ playlists.length ? "Pilih playlist di atas." : "Buat playlist pertama dengan mengetik namanya di atas." }}
            </template>
            <template v-else-if="view === 'playlists' && !query">Playlist ini masih kosong. Tambahkan lagu dari tab Lagu.</template>
            <template v-else>Tidak ada yang cocok dengan “{{ query }}”.</template>
          </p>
        </ul>
      </template>
    </main>
  </div>

  <section v-if="now && showNow" class="now" aria-label="Sedang diputar">
    <button class="icon-btn tonal close" @click="showNow = false" title="Tutup (Esc)"><Icon name="collapse" :size="28" /></button>
    <div class="stage">
      <div class="spin" :style="{ clipPath: 'url(#shape-now)' }">
        <img v-if="covers[now.path]" :src="covers[now.path]!" :alt="`Sampul ${now.album}`" />
        <Icon v-else name="note" :size="120" />
      </div>
      <h2>{{ now.title }}</h2>
      <p>{{ unknown(now.artist) }} · {{ unknown(now.album) }}</p>
    </div>
    <div class="lyrics" :class="{ synced: lyricIdx >= 0 || lyrics[0]?.t != null }">
      <p v-for="(l, i) in lyrics" :key="i" :class="{ cur: i === lyricIdx, past: lyricIdx >= 0 && i < lyricIdx }">{{ l.text || "♪" }}</p>
      <div v-if="!lyrics.length" class="no-lyrics">
        <Icon name="lyrics" :size="40" />
        <p>Lirik tidak tersedia</p>
        <span>Tidak ditemukan di LRCLIB. Letakkan file <code>.lrc</code> dengan nama yang sama di samping lagu.</span>
      </div>
    </div>
  </section>

  <footer v-if="now" class="player">
    <button class="np" @click="showNow = !showNow" :title="showNow ? 'Tutup layar putar' : 'Buka layar putar'">
      <CoverArt :path="now.path" />
      <span class="meta">
        <strong>{{ now.title }}</strong>
        <span>{{ unknown(now.artist) }}</span>
      </span>
    </button>
    <div class="ctrl">
      <div class="transport">
        <button class="icon-btn" :class="{ on: shuffle }" @click="shuffle = !shuffle" title="Acak" :aria-pressed="shuffle"><Icon name="shuffle" /></button>
        <button class="icon-btn tonal" @click="prev" title="Sebelumnya"><Icon name="prev" :size="28" /></button>
        <button class="play" :class="{ playing }" @click="togglePause" :title="paused ? 'Putar (Spasi)' : 'Jeda (Spasi)'">
          <Icon :name="paused ? 'play' : 'pause'" :size="32" />
        </button>
        <button class="icon-btn tonal" @click="next()" title="Berikutnya"><Icon name="next" :size="28" /></button>
        <button class="icon-btn" :class="{ on: repeat !== 'off' }" @click="cycleRepeat" :title="`Ulangi: ${repeatLabel}`" :aria-pressed="repeat !== 'off'">
          <Icon :name="repeat === 'one' ? 'repeatOne' : 'repeat'" />
        </button>
      </div>
      <div class="seek">
        <span>{{ fmt(position) }}</span>
        <WavyProgress :value="position" :max="now.duration_secs" :playing="playing" @scrub="scrub" @seek="seekTo" />
        <span>{{ fmt(now.duration_secs) }}</span>
      </div>
    </div>
    <div class="side">
      <button class="icon-btn" :class="{ on: showNow }" @click="showNow = !showNow" title="Lirik" :aria-pressed="showNow"><Icon name="lyrics" /></button>
      <Icon name="volume" />
      <input class="slider" type="range" min="0" max="1" step="0.01" v-model.number="volume"
        :style="{ '--p': `${volume * 100}%` }" aria-label="Volume"
        @input="invoke('set_volume', { volume })" />
    </div>
  </footer>
</template>

<style scoped>
.defs { position: absolute; width: 0; height: 0; }

/* ---------- layout ---------- */
.app {
  display: grid; grid-template-columns: var(--sidebar) 1fr;
  height: calc(100vh - var(--titlebar) - var(--sheet-gap));
  margin: var(--titlebar) var(--sheet-gap) 0; overflow: hidden;
  border-radius: var(--sheet-radius) var(--sheet-radius) 0 0; background: var(--surface);
}
main { overflow-y: auto; padding: 12px 40px 40px 16px; scrollbar-width: thin; scrollbar-color: var(--outline-variant) transparent; }
main.has-player { padding-bottom: 136px; }

/* ---------- navigation drawer ---------- */
.drawer { display: flex; flex-direction: column; gap: 16px; padding: 12px 12px 20px; overflow-y: auto; }
.drawer.has-player { padding-bottom: 136px; } /* keep the last item clear of the full-width player */
.fab {
  align-self: stretch; display: flex; align-items: center; gap: 12px;
  height: 56px; padding: 0 20px 0 16px; border-radius: 16px; font-size: 15px; font-weight: 600;
  background: var(--primary-container); color: var(--on-primary-container);
  box-shadow: 0 2px 6px rgb(0 0 0 / 0.25);
  transition: border-radius 0.35s var(--spring), transform 0.35s var(--spring);
}
.fab:hover { border-radius: 28px; }
.fab:active { transform: scale(0.95); }
.fab:disabled { cursor: default; }
.dest { display: flex; flex-direction: column; gap: 2px; margin-top: 4px; }
.dest-item {
  display: flex; align-items: center; gap: 12px; height: 56px; padding: 0 16px; border-radius: 28px;
  font-size: 15px; font-weight: 600; color: var(--on-surface-variant); text-align: left;
  transition: background 0.25s var(--emph), color 0.25s var(--emph);
}
.dest-item:hover { background: var(--surface-container-high); color: var(--on-surface); }
.dest-item.sel { background: var(--secondary-container); color: var(--on-secondary-container); }
.badge { margin-left: auto; font-size: 13px; font-variant-numeric: tabular-nums; opacity: 0.8; }

/* ---------- header ---------- */
.top { display: flex; align-items: flex-end; justify-content: space-between; gap: 24px; margin-bottom: 24px; }
.heading { display: flex; align-items: center; gap: 16px; min-width: 0; }
h1 {
  margin: 0; font-size: clamp(40px, 5vw, 64px); line-height: 1; letter-spacing: -0.03em;
  font-weight: 800; font-variation-settings: "wdth" 118, "opsz" 72;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.sub { margin: 8px 0 0; color: var(--on-surface-variant); font-size: 15px; font-weight: 500; }
.search {
  display: flex; align-items: center; gap: 12px; height: 56px; padding: 0 8px 0 18px; width: min(360px, 40%);
  border-radius: 28px; background: var(--surface-container-high); color: var(--on-surface-variant);
  transition: background 0.2s;
}
.search:focus-within { background: var(--surface-container-highest); }
.search input { flex: 1; min-width: 0; border: 0; background: none; outline: none; font-size: 16px; color: var(--on-surface); }
.search input::placeholder { color: var(--on-surface-variant); }

/* ---------- buttons ---------- */
.btn-group { display: flex; flex-wrap: wrap; gap: 8px; }
.actions { margin-bottom: 20px; }
.btn {
  display: inline-flex; align-items: center; gap: 8px; height: 48px; padding: 0 24px 0 18px;
  border-radius: 24px; font-size: 15px; font-weight: 600;
  transition: border-radius 0.3s var(--spring), background 0.2s;
}
.btn.big { height: 64px; padding: 0 32px 0 24px; border-radius: 32px; font-size: 18px; font-variation-settings: "wdth" 110; }
.btn:active { border-radius: 12px; }
.btn:disabled { opacity: 0.5; cursor: default; }
.filled { background: var(--primary); color: var(--on-primary); }
.tonal { background: var(--secondary-container); color: var(--on-secondary-container); }
.icon-btn {
  width: 40px; height: 40px; border-radius: 20px; display: inline-grid; place-items: center; flex: none;
  color: var(--on-surface-variant); transition: background 0.2s, border-radius 0.3s var(--spring);
}
.icon-btn:hover { background: var(--surface-container-highest); }
.icon-btn.tonal { color: var(--on-secondary-container); }
.icon-btn:active { border-radius: 10px; }
.icon-btn.on { color: var(--on-primary-container); background: var(--primary-container); }

/* ---------- grid ---------- */
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(176px, 1fr)); gap: 28px 20px; }
.tile { display: flex; flex-direction: column; gap: 4px; text-align: left; min-width: 0; }
.tile .art { width: 100%; height: auto; aspect-ratio: 1; margin-bottom: 10px; transition: transform 0.45s var(--spring); }
.tile:hover .art { transform: scale(1.05) rotate(-2deg); }
.tile strong { font-size: 16px; font-weight: 650; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.tile > span { font-size: 14px; color: var(--on-surface-variant); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.grid.artists .tile { align-items: center; text-align: center; }
.grid.artists .tile strong, .grid.artists .tile > span { max-width: 100%; }

/* ---------- song list ---------- */
.songs { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 2px; }
.songs li { display: flex; align-items: center; gap: 8px; padding-right: 16px; border-radius: 16px; transition: background 0.2s; }
.songs li:hover { background: var(--surface-container); }
.songs li.active { background: var(--secondary-container); color: var(--on-secondary-container); }
.row { flex: 1; min-width: 0; display: flex; align-items: center; gap: 16px; padding: 8px 12px; text-align: left; border-radius: 16px; cursor: default; }
.art-wrap { position: relative; flex: none; }
.art-overlay { position: absolute; inset: 0; display: grid; place-items: center; border-radius: 12px; background: rgb(0 0 0 / 0.45); color: #fff; }
.meta { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
.meta strong { font-size: 16px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.meta span { font-size: 14px; color: var(--on-surface-variant); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
li.active .meta span { color: inherit; opacity: 0.8; }
.dur { font-size: 14px; color: var(--on-surface-variant); font-variant-numeric: tabular-nums; min-width: 3.5ch; text-align: right; }
li.active .dur { color: inherit; }
.row-act { position: relative; opacity: 0; transition: opacity 0.15s; }
li:hover .row-act, .row-act:focus-visible, .row-menu.open .row-act { opacity: 1; }
.row-menu { position: relative; }
.pl-menu {
  position: absolute; right: 0; top: calc(100% + 4px); z-index: 5; min-width: 200px; max-height: 280px; overflow-y: auto;
  display: flex; flex-direction: column; padding: 8px 0; border-radius: 16px;
  background: var(--surface-container-high); box-shadow: 0 4px 16px rgb(0 0 0 / 0.35);
}
.pl-menu p { margin: 0; padding: 4px 16px 8px; font-size: 12px; font-weight: 600; color: var(--on-surface-variant); }
.pl-menu button { padding: 10px 16px; text-align: left; font-size: 14px; }
.pl-menu button:hover, .pl-menu button:focus-visible { background: var(--surface-container-highest); }
.none { color: var(--on-surface-variant); font-size: 15px; padding: 24px 12px; }

.eq { display: inline-flex; align-items: flex-end; gap: 3px; height: 18px; }
.eq i { width: 4px; height: 100%; border-radius: 2px; background: currentColor; transform-origin: bottom; animation: eq 0.9s ease-in-out infinite alternate; }
.eq i:nth-child(2) { animation-delay: -0.3s; }
.eq i:nth-child(3) { animation-delay: -0.6s; }
.eq.still i { animation-play-state: paused; }
@keyframes eq { from { transform: scaleY(0.25); } to { transform: scaleY(1); } }

/* ---------- chips ---------- */
.chips { display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 20px; }
.chip {
  display: inline-flex; align-items: center; height: 40px; border-radius: 12px;
  border: 1px solid var(--outline-variant); color: var(--on-surface-variant);
  transition: background 0.2s, border-radius 0.3s var(--spring);
}
.chip > button:first-child { height: 100%; padding: 0 16px; font-size: 14px; font-weight: 600; }
.chip:hover { background: var(--surface-container-high); }
.chip.sel { background: var(--secondary-container); color: var(--on-secondary-container); border-color: transparent; border-radius: 20px; }
.chip.sel > button:first-child { padding-right: 4px; }
.chip-x { width: 32px; height: 32px; margin-right: 4px; border-radius: 16px; display: grid; place-items: center; }
.chip.new { gap: 6px; padding: 0 6px 0 12px; border-style: dashed; }
.chip.new input { width: 130px; border: 0; background: none; outline: none; font-size: 14px; }

/* ---------- empty state ---------- */
.empty { display: flex; flex-direction: column; align-items: center; text-align: center; padding: 8vh 0; }
.empty-mark {
  width: 200px; height: 200px; display: grid; place-items: center; margin-bottom: 28px;
  background: var(--primary-container); color: var(--on-primary-container);
  animation: spin 24s linear infinite;
}
.empty-mark .icon { animation: spin 24s linear infinite reverse; }
.empty h2 { margin: 0 0 8px; font-size: 32px; font-weight: 750; letter-spacing: -0.02em; font-variation-settings: "wdth" 115; }
.empty p { margin: 0 0 28px; color: var(--on-surface-variant); max-width: 42ch; font-size: 16px; line-height: 1.5; }

.scanning { display: flex; align-items: center; gap: 12px; color: var(--on-surface-variant); margin: 0 0 20px 12px; }
.spinner { width: 24px; height: 24px; background: var(--primary); animation: spin 1.2s linear infinite, pulse 1.2s var(--emph) infinite alternate; }
.fab .spinner { background: var(--on-primary-container); }
@keyframes spin { to { transform: rotate(360deg); } }
@keyframes pulse { from { scale: 0.8; } to { scale: 1.05; } }

/* ---------- S3 panel ---------- */
.panel { max-width: 640px; padding: 28px; border-radius: 28px; background: var(--surface-container-low); display: flex; flex-direction: column; gap: 20px; }
.hint { margin: 0; color: var(--on-surface-variant); line-height: 1.5; }
.fields { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
.field { display: flex; flex-direction: column; gap: 6px; font-size: 13px; font-weight: 600; color: var(--on-surface-variant); }
.field.wide { grid-column: 1 / -1; }
.field input {
  height: 52px; padding: 0 16px; border: 0; border-radius: 12px 12px 4px 4px; outline: none;
  background: var(--surface-container-highest); color: var(--on-surface); font-size: 15px; font-weight: 400;
  box-shadow: inset 0 -1px 0 var(--on-surface-variant); transition: box-shadow 0.2s;
}
.field input:focus { box-shadow: inset 0 -3px 0 var(--primary); }
.field input::placeholder { color: var(--on-surface-variant); opacity: 0.7; }
.status { margin: 0; padding: 12px 16px; border-radius: 12px; background: var(--secondary-container); color: var(--on-secondary-container); }
.status.err { background: #8c1d18; color: #ffdad6; }

/* ---------- player bar ---------- */
.player {
  position: fixed; left: 16px; right: 16px; bottom: 16px; z-index: 20;
  display: grid; grid-template-columns: minmax(180px, 1fr) minmax(320px, 1.6fr) minmax(180px, 1fr);
  align-items: center; gap: 24px; padding: 12px 20px 12px 12px;
  border-radius: 28px; background: var(--surface-container-high);
  /* the surface-coloured spread masks list rows scrolling past the floating bar */
  box-shadow: 0 6px 24px rgb(0 0 0 / 0.28), 0 0 0 calc(16px - var(--sheet-gap)) var(--surface);
}
.np { display: flex; align-items: center; gap: 14px; min-width: 0; text-align: left; border-radius: 16px; padding-right: 8px; }
.np .art { width: 64px; height: 64px; border-radius: 16px; transition: border-radius 0.4s var(--spring); }
.np:hover .art { border-radius: 32px; }
.ctrl { display: flex; flex-direction: column; align-items: stretch; gap: 4px; }
.transport { display: flex; align-items: center; justify-content: center; gap: 8px; }
.transport .icon-btn.tonal { width: 48px; height: 48px; border-radius: 24px; background: var(--secondary-container); }
.transport .icon-btn.tonal:active { border-radius: 12px; }
.play {
  width: 76px; height: 56px; border-radius: 28px; display: grid; place-items: center;
  background: var(--primary); color: var(--on-primary);
  transition: border-radius 0.45s var(--spring), transform 0.3s var(--spring);
}
.play.playing { border-radius: 18px; }
.play:active { transform: scale(0.94); }
.seek { display: flex; align-items: center; gap: 12px; font-size: 12px; font-weight: 600; color: var(--on-surface-variant); font-variant-numeric: tabular-nums; }
.side { display: flex; align-items: center; justify-content: flex-end; gap: 10px; color: var(--on-surface-variant); }
.slider {
  appearance: none; -webkit-appearance: none; width: 120px; height: 16px; border-radius: 8px; margin: 0; cursor: pointer;
  background: linear-gradient(to right, var(--primary) var(--p), var(--secondary-container) var(--p));
}
.slider::-webkit-slider-thumb {
  -webkit-appearance: none; width: 4px; height: 32px; border-radius: 2px; background: var(--primary);
  box-shadow: 0 0 0 4px var(--surface-container-high);
}
.slider::-moz-range-thumb { width: 4px; height: 32px; border: 0; border-radius: 2px; background: var(--primary); }

/* ---------- now playing ---------- */
.now {
  position: fixed; inset: var(--titlebar) var(--sheet-gap) var(--sheet-gap); z-index: 10;
  display: grid; grid-template-columns: 1fr 1.1fr; gap: 48px;
  padding: 40px 64px 132px; background: var(--surface); border-radius: var(--sheet-radius) var(--sheet-radius) 0 0;
  animation: rise 0.5s var(--emph);
}
@keyframes rise { from { opacity: 0; transform: translateY(32px); } }
.close { position: absolute; top: 12px; left: 12px; width: 48px; height: 48px; border-radius: 24px; background: var(--secondary-container); }
.stage { display: flex; flex-direction: column; justify-content: center; align-items: center; text-align: center; min-width: 0; }
.spin {
  width: min(52vh, 34vw); aspect-ratio: 1; display: grid; place-items: center; overflow: hidden;
  background: var(--primary-container); color: var(--on-primary-container);
}
.spin img { width: 100%; height: 100%; object-fit: cover; }
.stage h2 {
  margin: 32px 0 8px; font-size: clamp(28px, 3.4vw, 48px); line-height: 1.05; letter-spacing: -0.03em;
  font-weight: 800; font-variation-settings: "wdth" 120, "opsz" 60; text-wrap: balance;
}
.stage p { margin: 0; color: var(--primary); font-size: 18px; font-weight: 600; }
.lyrics {
  overflow-y: auto; padding: 30vh 8px; scrollbar-width: none;
  mask-image: linear-gradient(transparent, #000 22%, #000 78%, transparent);
}
.lyrics p {
  margin: 0 0 18px; font-size: clamp(22px, 2.2vw, 32px); line-height: 1.25; font-weight: 750;
  letter-spacing: -0.02em; color: var(--on-surface); opacity: 0.35;
  transform-origin: left center; transition: opacity 0.35s, transform 0.5s var(--spring), color 0.35s;
}
.lyrics:not(.synced) p { opacity: 0.85; font-size: 20px; font-weight: 500; }
.lyrics p.past { opacity: 0.2; }
.lyrics p.cur { opacity: 1; color: var(--primary); transform: scale(1.04); }
.no-lyrics { height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; color: var(--on-surface-variant); text-align: center; }
.lyrics .no-lyrics p { opacity: 1; font-size: 22px; color: var(--on-surface); margin: 8px 0 0; }
.no-lyrics code { background: var(--surface-container-high); padding: 2px 6px; border-radius: 6px; }

@media (max-width: 1100px) {
  .player { grid-template-columns: minmax(0, 1fr) auto; }
  .side { display: none; }
  .now { grid-template-columns: 1fr; padding: 56px 32px 132px; }
  .lyrics { display: none; }
}
@media (prefers-reduced-motion: reduce) {
  .empty-mark, .empty-mark .icon, .eq i { animation: none; }
}
</style>
