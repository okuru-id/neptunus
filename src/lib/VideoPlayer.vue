<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import Icon from "./Icon.vue";
import { subtitles, type Subtitle, type SubtitleQuery } from "./videoext";

const props = defineProps<{ path: string; title: string; sub?: SubtitleQuery }>();
const emit = defineEmits<{ close: [] }>();

const ext = (p: string) => p.split(/[?#]/)[0].split(".").pop()?.toLowerCase() ?? "";

const video = ref<HTMLVideoElement>();
const error = ref("");
// where each video was left, so it picks up from there next time
const posKey = `video-pos:${props.path}`;
let hls: { destroy(): void } | undefined;
const buffering = ref(true);
let lastSave = 0;
let resolvedUrl = "";

async function openVlc() {
  try {
    await invoke("video_open_external", { path: resolvedUrl || props.path });
  } catch (e: any) {
    error.value = typeof e === "string" ? e : `VLC gagal dibuka (${e?.message ?? e}).`;
  }
}

function savePos(force = false) {
  const v = video.value;
  if (!v || (!force && Date.now() - lastSave < 5000)) return;
  lastSave = Date.now();
  try {
    if (v.duration && v.duration - v.currentTime > 10) localStorage.setItem(posKey, String(v.currentTime));
    else localStorage.removeItem(posKey);
  } catch {}
}

// --- controls: our own bar instead of the webview's, so it looks the same everywhere ---
const box = ref<HTMLElement>();
const playing = ref(false);
const time = ref(0);
const duration = ref(0);
const buffered = ref(0);
const volume = ref(1);
const muted = ref(false);
const full = ref(false);
const idle = ref(false);
let idleTimer: ReturnType<typeof setTimeout> | undefined;

const fmt = (t: number) => {
  if (!isFinite(t)) return "0:00";
  const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), s = Math.floor(t % 60);
  return `${h ? `${h}:${String(m).padStart(2, "0")}` : m}:${String(s).padStart(2, "0")}`;
};
const pct = (n: number) => `${duration.value ? Math.min(100, (n / duration.value) * 100) : 0}%`;

function sync() {
  const v = video.value;
  if (!v) return;
  time.value = v.currentTime;
  duration.value = isFinite(v.duration) ? v.duration : 0;
  for (let i = 0; i < v.buffered.length; i++)
    if (v.buffered.start(i) <= v.currentTime + 0.5) buffered.value = v.buffered.end(i);
}
function wake() {
  idle.value = false;
  clearTimeout(idleTimer);
  idleTimer = setTimeout(() => { if (playing.value && !menu.value) idle.value = true; }, 2800);
}
const toggle = () => { const v = video.value!; v.paused ? v.play().catch(() => {}) : v.pause(); };
const skip = (d: number) => { const v = video.value!; v.currentTime = Math.max(0, Math.min(v.duration || 0, v.currentTime + d)); wake(); };
const seek = (e: Event) => { video.value!.currentTime = Number((e.target as HTMLInputElement).value); };
function setVolume(e: Event) {
  const v = video.value!;
  v.volume = Number((e.target as HTMLInputElement).value);
  v.muted = v.volume === 0;
}
const toggleMute = () => { const v = video.value!; v.muted = !v.muted; if (!v.muted && !v.volume) v.volume = 0.5; };
const toggleFull = () => (document.fullscreenElement ? document.exitFullscreen() : box.value?.requestFullscreen())?.catch(() => {});
const onFull = () => (full.value = !!document.fullscreenElement);

// --- subtitles from the subtitle addons, rendered by us above the bar ---
const SUB_LANG = "neptunus.subLang";
const subs = ref<Subtitle[] | null>(null);
const subPick = ref<Subtitle | null>(null);
const subLoading = ref(false);
const cue = ref("");
const menu = ref(false);
let subBlob = "";
const langName = (l?: string) => {
  if (!l) return "Lainnya";
  try { return new Intl.DisplayNames(["id"], { type: "language" }).of(l.length === 3 ? ({ ind: "id", eng: "en", may: "ms", jpn: "ja", kor: "ko", chi: "zh", zho: "zh", spa: "es", fre: "fr", fra: "fr", ger: "de", deu: "de", por: "pt", ara: "ar", tha: "th", vie: "vi", rus: "ru", ita: "it", hin: "hi" }[l] ?? l) : l) ?? l; } catch { return l; }
};
// one entry per language, numbered when an addon offers several files for it
const subList = computed(() => {
  const seen: Record<string, number> = {};
  return [...(subs.value ?? [])]
    .sort((a, b) => langName(a.lang).localeCompare(langName(b.lang)))
    .map((s) => { const n = (seen[s.lang ?? ""] = (seen[s.lang ?? ""] ?? 0) + 1); return { s, label: `${langName(s.lang)}${n > 1 ? ` ${n}` : ""}` }; });
});

async function pickSub(s: Subtitle | null) {
  menu.value = false;
  subPick.value = s;
  cue.value = "";
  const v = video.value!;
  v.querySelectorAll("track").forEach((t) => t.remove());
  if (subBlob) URL.revokeObjectURL(subBlob);
  subBlob = "";
  try { s ? localStorage.setItem(SUB_LANG, s.lang ?? "") : localStorage.removeItem(SUB_LANG); } catch {}
  if (!s) return;
  subLoading.value = true;
  try {
    const vtt = await invoke<string>("video_subtitle", { url: s.url });
    if (subPick.value !== s) return;
    subBlob = URL.createObjectURL(new Blob([vtt], { type: "text/vtt" }));
    const t = document.createElement("track");
    t.kind = "subtitles";
    t.src = subBlob;
    v.appendChild(t);
    t.track.mode = "hidden"; // cues come to us, the webview draws nothing
    t.track.addEventListener("cuechange", () => {
      cue.value = [...(t.track.activeCues ?? [])].map((c) => (c as VTTCue).text.replace(/<[^>]+>/g, "")).join("\n");
    });
  } catch (e) {
    error.value = `Subtitle tidak bisa dimuat (${e}).`;
    subPick.value = null;
  } finally {
    subLoading.value = false;
  }
}

async function loadSubs() {
  if (!props.sub?.sources.length) return;
  subs.value = await subtitles(props.sub);
  let lang: string | null = null;
  try { lang = localStorage.getItem(SUB_LANG); } catch {}
  const again = lang !== null && subs.value.find((s) => (s.lang ?? "") === lang);
  if (again) pickSub(again);
}

function onKey(e: KeyboardEvent) {
  if ((e.target as HTMLElement)?.closest?.("input:not([type=range]), textarea")) return;
  if (e.key === "Escape" && menu.value) { menu.value = false; return; }
  if (e.key === "Escape" && !document.fullscreenElement) emit("close");
  else if (e.key === " " || e.key === "k") { e.preventDefault(); toggle(); }
  else if (e.key === "ArrowLeft") skip(-10);
  else if (e.key === "ArrowRight") skip(10);
  else if (e.key === "f") toggleFull();
  else if (e.key === "m") toggleMute();
  else return;
  wake();
}

onMounted(async () => {
  window.addEventListener("keydown", onKey);
  document.addEventListener("fullscreenchange", onFull);
  wake();
  loadSubs();
  try {
    const url = props.path.startsWith("http")
      ? props.path
      : (await invoke<string | null>("video_url", { path: props.path })) ?? convertFileSrc(props.path);
    resolvedUrl = url;
    const v = video.value!;
    let start: number | null = null;
    try { start = Number(localStorage.getItem(posKey)) || null; } catch {}
    if (start) v.addEventListener("loadedmetadata", () => { v.currentTime = start; }, { once: true });
    const fail = (why: string) => {
      error.value = `Format video ini tidak didukung di perangkat ini. Coba putar di VLC. [${why}]`;
      buffering.value = false;
    };
    // the system player first: GStreamer (Linux), WebKit (Apple) and Android play HLS themselves and,
    // unlike hls.js, fetch segments without CORS. hls.js only steps in when that fails, which in
    // practice is WebView2 on Windows.
    v.addEventListener("error", async () => {
      const native = `${v.error?.code ?? "?"}${v.error?.message ? `: ${v.error.message}` : ""}`;
      if (ext(props.path) === "m3u8" && !hls) {
        const Hls = (await import("hls.js")).default;
        if (Hls.isSupported()) {
          const h = new Hls({ maxBufferLength: 60, maxMaxBufferLength: 180, backBufferLength: 30, startPosition: start ?? -1 });
          hls = h;
          h.on(Hls.Events.ERROR, (_, d) => d.fatal && fail(`sistem ${native} · hls.js ${d.details}${d.response?.code ? ` ${d.response.code}` : ""}`));
          h.loadSource(url);
          h.attachMedia(v);
          return;
        }
      }
      fail(native);
    });
    v.src = url;
  } catch (e: any) {
    error.value = typeof e === "string" ? e : `Video tidak bisa diputar (${e?.message ?? e}).`;
    buffering.value = false;
  }
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKey);
  document.removeEventListener("fullscreenchange", onFull);
  clearTimeout(idleTimer);
  if (subBlob) URL.revokeObjectURL(subBlob);
  savePos(true);
  hls?.destroy();
});
</script>

<template>
  <div ref="box" class="vplayer" :class="{ idle }" role="dialog" :aria-label="title" @pointermove="wake" @pointerdown="wake">
    <video ref="video" autoplay playsinline preload="auto" @click="toggle" @dblclick="toggleFull"
      @timeupdate="sync(); savePos()" @progress="sync" @durationchange="sync" @pause="playing = false; idle = false; savePos(true)" @play="playing = true; wake()"
      @ended="savePos(true)" @volumechange="volume = video!.volume; muted = video!.muted"
      @waiting="buffering = true" @loadstart="buffering = true" @playing="buffering = false" @canplay="buffering = false" @seeked="buffering = false"></video>
    <div v-if="buffering && !error" class="spin" aria-label="Memuat"></div>
    <p v-if="cue" class="cue">{{ cue }}</p>
    <p v-if="error" class="err">{{ error }}</p>

    <div class="bar top">
      <button title="Tutup (Esc)" @click="emit('close')"><Icon name="back" /></button>
      <strong>{{ title }}</strong>
      <button class="vlc" title="Buka di VLC" @click="openVlc"><Icon name="openInNew" /></button>
    </div>

    <div class="bar bottom" @pointerdown.stop>
      <div class="seek" :style="{ '--done': pct(time), '--buf': pct(buffered) }">
        <input type="range" min="0" :max="duration || 0" step="0.1" :value="time" aria-label="Posisi" @input="seek" />
      </div>
      <div class="row">
        <button class="main" :title="playing ? 'Jeda (Spasi)' : 'Putar (Spasi)'" @click="toggle"><Icon :name="playing ? 'pause' : 'play'" :size="30" /></button>
        <button title="Mundur 10 detik (←)" @click="skip(-10)"><Icon name="replay10" /></button>
        <button title="Maju 10 detik (→)" @click="skip(10)"><Icon name="forward10" /></button>
        <div class="vol">
          <button :title="muted ? 'Bunyikan (M)' : 'Bisukan (M)'" @click="toggleMute"><Icon :name="muted || !volume ? 'volumeOff' : 'volume'" /></button>
          <input type="range" min="0" max="1" step="0.05" :value="muted ? 0 : volume" aria-label="Volume" :style="{ '--done': `${(muted ? 0 : volume) * 100}%` }" @input="setVolume" />
        </div>
        <span class="time">{{ fmt(time) }} <i>/ {{ fmt(duration) }}</i></span>
        <span class="grow"></span>
        <div v-if="sub?.sources.length" class="subwrap">
          <button :class="{ on: subPick }" :title="subs === null ? 'Mencari subtitle…' : 'Subtitle'" @click="menu = !menu"><Icon name="subtitles" /></button>
          <div v-if="menu" class="menu" role="menu">
            <p class="mhead">Subtitle</p>
            <button role="menuitemradio" :aria-checked="!subPick" :class="{ on: !subPick }" @click="pickSub(null)"><Icon name="done" :size="18" />Mati</button>
            <p v-if="subs === null" class="mnote">Mencari subtitle…</p>
            <p v-else-if="!subs.length" class="mnote">Tidak ada subtitle untuk judul ini.</p>
            <button v-for="x in subList" :key="x.s.url" role="menuitemradio" :aria-checked="subPick === x.s" :class="{ on: subPick === x.s }"
              :title="x.s.source" @click="pickSub(x.s)">
              <Icon name="done" :size="18" />{{ x.label }}<small v-if="subPick === x.s && subLoading">memuat…</small>
            </button>
          </div>
        </div>
        <button :title="full ? 'Keluar layar penuh (F)' : 'Layar penuh (F)'" @click="toggleFull"><Icon :name="full ? 'fullscreenExit' : 'fullscreen'" /></button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.vplayer {
  position: fixed; inset: var(--titlebar) 0 0; z-index: 30; background: #000; color: #fff;
  display: flex; align-items: center; justify-content: center;
}
.vplayer:fullscreen { inset: 0; }
.vplayer.idle { cursor: none; }
/* pinned to the box: a grid row sized by the video itself let tall frames spill under the window edge */
video { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; background: #000; }

.bar {
  position: absolute; left: 0; right: 0; display: flex; gap: 8px;
  padding-left: max(16px, env(safe-area-inset-left)); padding-right: max(16px, env(safe-area-inset-right));
  transition: opacity 0.3s, transform 0.35s var(--spring);
}
.idle .bar { opacity: 0; pointer-events: none; }
.idle .top { transform: translateY(-12px); }
.idle .bottom { transform: translateY(12px); }
.top { top: 0; align-items: center; padding-top: 10px; padding-bottom: 36px; background: linear-gradient(rgb(0 0 0 / 0.7), transparent); }
.top strong { flex: 1; font-size: 17px; font-weight: 650; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.bottom {
  bottom: 0; flex-direction: column; gap: 4px; padding-top: 48px; padding-bottom: max(12px, env(safe-area-inset-bottom));
  background: linear-gradient(transparent, rgb(0 0 0 / 0.8));
}
.row { display: flex; align-items: center; gap: 4px; }
.grow { flex: 1; }
.bar button {
  color: inherit; width: 44px; height: 44px; border-radius: 22px; display: inline-grid; place-items: center; flex: none;
  transition: background 0.2s, border-radius 0.3s var(--spring), transform 0.3s var(--spring);
}
.bar button:hover { background: rgb(255 255 255 / 0.14); }
.bar button:active { border-radius: 12px; transform: scale(0.94); }
.bar button.on { color: var(--primary, #a8c7fa); }
.bar .main { width: 52px; height: 52px; border-radius: 26px; background: #fff; color: #000; margin-right: 6px; }
.bar .main:hover { background: rgb(255 255 255 / 0.85); }
.bar .main:active { border-radius: 14px; }
.time { font-size: 14px; font-variant-numeric: tabular-nums; margin-left: 8px; white-space: nowrap; }
.time i { font-style: normal; opacity: 0.65; }

/* seek: played / buffered / rest, with a thumb that grows on hover */
input[type=range] { -webkit-appearance: none; appearance: none; background: none; margin: 0; cursor: pointer; }
.seek { position: relative; height: 20px; display: flex; align-items: center; }
.seek input { width: 100%; height: 20px; }
.seek input::-webkit-slider-runnable-track {
  height: 4px; border-radius: 2px; transition: height 0.15s;
  background: linear-gradient(to right, #fff var(--done), rgb(255 255 255 / 0.45) var(--done) var(--buf), rgb(255 255 255 / 0.2) var(--buf));
}
.seek:hover input::-webkit-slider-runnable-track { height: 6px; }
.seek input::-moz-range-track { height: 4px; border-radius: 2px; background: linear-gradient(to right, #fff var(--done), rgb(255 255 255 / 0.45) var(--done) var(--buf), rgb(255 255 255 / 0.2) var(--buf)); }
input::-webkit-slider-thumb {
  -webkit-appearance: none; width: 14px; height: 14px; border-radius: 50%; background: #fff; margin-top: -5px;
  transform: scale(0); transition: transform 0.2s var(--spring);
}
.seek:hover input::-webkit-slider-thumb { transform: scale(1); margin-top: -4px; }
input::-moz-range-thumb { width: 14px; height: 14px; border: 0; border-radius: 50%; background: #fff; }
.vol { display: flex; align-items: center; }
.vol input { width: 0; height: 20px; opacity: 0; transition: width 0.3s var(--spring), opacity 0.2s; }
.vol:hover input, .vol:focus-within input { width: 90px; opacity: 1; margin-right: 6px; }
.vol input::-webkit-slider-runnable-track { height: 4px; border-radius: 2px; background: linear-gradient(to right, #fff var(--done), rgb(255 255 255 / 0.25) var(--done)); }
.vol input::-webkit-slider-thumb { transform: scale(1); }

.subwrap { position: relative; }
.menu {
  position: absolute; right: 0; bottom: 52px; min-width: 220px; max-height: min(360px, 60vh); overflow-y: auto; padding: 8px;
  border-radius: 20px; background: rgb(28 28 30 / 0.94); backdrop-filter: blur(16px); box-shadow: 0 8px 32px rgb(0 0 0 / 0.5);
  display: flex; flex-direction: column; scrollbar-width: thin;
}
.mhead { margin: 4px 12px 6px; font-size: 13px; font-weight: 700; opacity: 0.6; text-transform: uppercase; letter-spacing: 0.06em; }
.mnote { margin: 6px 12px; font-size: 14px; opacity: 0.7; }
.menu button {
  width: 100%; height: 40px; border-radius: 12px; display: flex; gap: 10px; padding: 0 12px; justify-content: flex-start;
  font-size: 14px; text-align: left;
}
.menu button :deep(svg) { opacity: 0; }
.menu button.on :deep(svg) { opacity: 1; }
.menu small { margin-left: auto; opacity: 0.6; }

.cue {
  position: absolute; left: 50%; bottom: 7%; transform: translateX(-50%); max-width: min(90%, 1000px); margin: 0;
  padding: 4px 12px; border-radius: 8px; background: rgb(0 0 0 / 0.6); white-space: pre-line; text-align: center;
  font-size: clamp(16px, 2.4vw, 30px); line-height: 1.35; font-weight: 500; text-shadow: 0 1px 3px #000; pointer-events: none;
  transition: bottom 0.35s var(--spring);
}
.vplayer:not(.idle) .cue { bottom: 130px; }
.spin {
  position: absolute; width: 56px; height: 56px; border-radius: 50%; pointer-events: none;
  border: 5px solid rgb(255 255 255 / 0.25); border-top-color: #fff; animation: spin 0.9s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }
.err { position: absolute; background: rgb(0 0 0 / 0.75); padding: 12px 20px; border-radius: 16px; margin: 0 16px; text-align: center; }
@media (max-width: 600px) {
  .bar button { width: 40px; height: 40px; }
  .bar .main { width: 46px; height: 46px; }
  .vol input { display: none; }
  .time { font-size: 13px; margin-left: 4px; }
}
</style>
