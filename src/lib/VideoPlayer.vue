<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import Icon from "./Icon.vue";
import { subtitles, type Subtitle, type SubtitleQuery } from "./videoext";
import type Hls from "hls.js";

// `resume`: the film or episode id from a catalog, so where it was left holds whichever source plays it
const props = defineProps<{ path: string; title: string; sub?: SubtitleQuery; resume?: string }>();
const emit = defineEmits<{ close: [] }>();

const ext = (p: string) => p.split(/[?#]/)[0].split(".").pop()?.toLowerCase() ?? "";

const video = ref<HTMLVideoElement>();
const error = ref("");
// where each video was left, so it picks up from there next time
const posKey = `video-pos:${props.resume ?? props.path}`;
const durKey = `video-dur:${props.resume ?? props.path}`;
let hls: Hls | undefined;
const buffering = ref(true);
let lastSave = 0;
let closed = false; // the player is closing (or closed): late events and async steps do nothing
let lastFill = 0;
let filled = 0; // seconds the app has downloaded from the playhead on (`video_buffered`)
let resolvedUrl = "";

function savePos(force = false) {
  const v = video.value;
  // once closing, the element has let go of the video: no position to save, and none to forget
  if (!v || closed || (!force && Date.now() - lastSave < 5000)) return;
  lastSave = Date.now();
  try {
    if (v.duration && v.duration - v.currentTime > 10) {
      localStorage.setItem(posKey, String(v.currentTime));
      localStorage.setItem(durKey, String(v.duration)); // for the progress under "Lanjutkan menonton"
    } else {
      localStorage.removeItem(posKey);
      localStorage.removeItem(durKey);
    }
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
  if (!dragging) time.value = v.currentTime;
  duration.value = isFinite(v.duration) ? v.duration : 0;
  // the webview's own buffer, or what the app downloaded when that reaches further (kept between checks,
  // or every timeupdate would cut the bar back to the webview's few seconds)
  let own = 0;
  for (let i = 0; i < v.buffered.length; i++)
    if (v.buffered.start(i) <= v.currentTime + 0.5) own = v.buffered.end(i);
  buffered.value = Math.max(own, filled);
  // on phones the webview's player only buffers a little ahead; the app downloads the rest itself
  if (resolvedUrl.includes("/~fill/") && duration.value && Date.now() - lastFill > 1000) {
    lastFill = Date.now();
    const d = duration.value;
    invoke<number | null>("video_buffered", { at: v.currentTime / d })
      .then((f) => { filled = f == null ? 0 : f * d; buffered.value = Math.max(buffered.value, filled); })
      .catch(() => {});
  }
}
// touch: a tap on the picture shows the controls (another hides them) instead of pausing; the mouse still pauses
let touch = false;
let wasIdle = false;
function down(e: PointerEvent) {
  touch = e.pointerType !== "mouse";
  wasIdle = idle.value;
  wake();
}
function tap() {
  if (!touch) return toggle();
  if (wasIdle || !playing.value) return;
  clearTimeout(idleTimer);
  idle.value = true;
}
// sound but no picture: the video codec (often HEVC/AV1 10-bit) can't be decoded here while the audio can
let pictureTimer: ReturnType<typeof setTimeout> | undefined;
function checkPicture() {
  clearTimeout(pictureTimer);
  pictureTimer = setTimeout(() => {
    const v = video.value;
    if (v && !v.paused && v.currentTime > 1 && !v.videoWidth && !error.value)
      error.value = "Gambar video ini tidak bisa ditampilkan di perangkat ini (codec tidak didukung), hanya suaranya. Coba sumber lain, misalnya yang H.264/x264.";
  }, 3000);
}
function wake() {
  idle.value = false;
  clearTimeout(idleTimer);
  idleTimer = setTimeout(() => { if (playing.value && !menu.value) idle.value = true; }, 2800);
}
// the big icon that blinks in the middle on click, like YouTube's
const flash = ref<{ icon: "play" | "pause" | "replay10" | "forward10"; n: number } | null>(null);
const blink = (icon: "play" | "pause" | "replay10" | "forward10") => (flash.value = { icon, n: (flash.value?.n ?? 0) + 1 });
const toggle = () => {
  const v = video.value!;
  if (v.paused) { v.play().catch(() => {}); blink("play"); } else { v.pause(); blink("pause"); }
};

// seek bar: drag anywhere on it, with the time under the pointer shown above it
const seekEl = ref<HTMLElement>();
const hover = ref<number | null>(null);
let dragging = false;
const frac = (e: PointerEvent) => {
  const r = seekEl.value!.getBoundingClientRect();
  return Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
};
function seekDown(e: PointerEvent) {
  dragging = true;
  seekEl.value!.setPointerCapture(e.pointerId);
  seekMove(e);
}
function seekMove(e: PointerEvent) {
  hover.value = frac(e);
  if (dragging && duration.value) video.value!.currentTime = time.value = hover.value * duration.value;
  wake();
}
function seekKey(e: KeyboardEvent) {
  if (e.key === "ArrowLeft" || e.key === "ArrowRight") { e.stopPropagation(); skip(e.key === "ArrowLeft" ? -5 : 5); }
}
const skip = (d: number) => {
  const v = video.value!;
  v.currentTime = Math.max(0, Math.min(v.duration || 0, v.currentTime + d));
  blink(d < 0 ? "replay10" : "forward10");
  wake();
};
function setVolume(e: Event) {
  const v = video.value!;
  v.volume = Number((e.target as HTMLInputElement).value);
  v.muted = v.volume === 0;
}
const toggleMute = () => { const v = video.value!; v.muted = !v.muted; if (!v.muted && !v.volume) v.volume = 0.5; };
// Android's WebView ignores requestFullscreen, so the app's bridge hides the bars and rotates instead.
// iOS's WKWebView has element fullscreen turned off: the system video player goes fullscreen there.
const droid = (window as any).NeptunusAndroid as { fullscreen?(on: boolean): void } | undefined;
function toggleFull() {
  const v = video.value as (HTMLVideoElement & { webkitEnterFullscreen?(): void }) | undefined;
  if (droid?.fullscreen) droid.fullscreen((full.value = !full.value));
  else if (!document.fullscreenEnabled && v?.webkitEnterFullscreen) v.webkitEnterFullscreen();
  else (document.fullscreenElement ? document.exitFullscreen() : box.value?.requestFullscreen())?.catch(() => {});
}
const onFull = () => (full.value = !!document.fullscreenElement);

// --- subtitles from the subtitle addons, rendered by us above the bar ---
const subs = ref<Subtitle[] | null>(null);
const subPick = ref<Subtitle | null>(null);
const subLoading = ref(false);
const cue = ref("");
const menu = ref(false);
const langName = (l?: string) => {
  if (!l) return "Lainnya";
  try { return new Intl.DisplayNames(["id"], { type: "language" }).of(l.length === 3 ? ({ ind: "id", eng: "en", may: "ms", jpn: "ja", kor: "ko", chi: "zh", zho: "zh", spa: "es", fre: "fr", fra: "fr", ger: "de", deu: "de", por: "pt", ara: "ar", tha: "th", vie: "vi", rus: "ru", ita: "it", hin: "hi" }[l] ?? l) : l) ?? l; } catch { return l; }
};
// "Indonesia - Subtitle Translator", numbered when one addon offers several files for a language
const subList = computed(() => {
  const seen: Record<string, number> = {};
  return [...(subs.value ?? [])]
    .sort((a, b) => langName(a.lang).localeCompare(langName(b.lang)) || a.source.localeCompare(b.source))
    .map((s) => {
      const k = `${s.lang}|${s.source}`;
      const n = (seen[k] = (seen[k] ?? 0) + 1);
      return { s, label: `${langName(s.lang)}${n > 1 ? ` ${n}` : ""} - ${s.source}` };
    });
});

// --- audio tracks (dubbing): hls.js's list for HLS it plays, else the system player's ---
const audios = ref<string[]>([]);
const audioOn = ref(0);
const panelName = computed(() => (audios.value.length > 1 ? (props.sub?.sources.length ? "Audio & subtitle" : "Audio") : "Subtitle"));
function readAudios() {
  const list: { label?: string; name?: string; language?: string; lang?: string; enabled?: boolean }[] =
    hls ? hls.audioTracks : Array.from((video.value as any)?.audioTracks ?? []);
  audios.value = list.map((t, i) => {
    const name = t.label || t.name || (t.language || t.lang ? langName(t.language || t.lang) : `Audio ${i + 1}`);
    return list.some((o, j) => j !== i && (o.label || o.name) === (t.label || t.name) && (o.language || o.lang) === (t.language || t.lang)) ? `${name} ${i + 1}` : name;
  });
  audioOn.value = hls ? hls.audioTrack : Math.max(0, list.findIndex((t) => t.enabled));
}
function pickAudio(i: number) {
  if (hls) hls.audioTrack = i;
  else Array.from((video.value as any).audioTracks as { enabled: boolean }[]).forEach((t, j) => (t.enabled = j === i));
  audioOn.value = i;
}

// cues are kept and matched against the clock by us, so a delay is just an offset
let cues: { start: number; end: number; text: string }[] = [];
let raf = 0;
const secs = (t: string) => {
  const m = t.match(/(?:(\d+):)?(\d+):(\d+)[.,](\d+)/);
  return m ? Number(m[1] ?? 0) * 3600 + Number(m[2]) * 60 + Number(m[3]) + Number(m[4].padEnd(3, "0").slice(0, 3)) / 1000 : NaN;
};
function parseVtt(vtt: string) {
  return vtt.split(/\n{2,}/).flatMap((block) => {
    const lines = block.split("\n");
    const i = lines.findIndex((l) => l.includes("-->"));
    if (i < 0) return [];
    const [a, b] = lines[i].split("-->");
    const text = lines.slice(i + 1).join("\n").replace(/<[^>]+>|\{\\[^}]*\}/g, "").trim();
    const start = secs(a), end = secs(b);
    return text && start < end ? [{ start, end, text }] : [];
  });
}
// iOS's fullscreen is the system player, which can't show our overlay: hand it the cues (with the
// delay applied) as a text track while it is up
let nativeTrack: TextTrack | undefined;
function nativeSubs(on: boolean) {
  const v = video.value;
  if (!v || (!on && !nativeTrack)) return;
  const t = (nativeTrack ??= v.addTextTrack("subtitles", "Subtitle"));
  t.mode = "hidden";
  for (const c of Array.from(t.cues ?? [])) t.removeCue(c);
  if (on) for (const c of cues) t.addCue(new VTTCue(c.start + subDelay.value, c.end + subDelay.value, c.text));
  t.mode = on && cues.length ? "showing" : "disabled";
}
function tick() {
  const t = (video.value?.currentTime ?? 0) - subDelay.value;
  const now = cues.filter((c) => c.start <= t && t < c.end).map((c) => c.text).join("\n");
  if (now !== cue.value) cue.value = now;
  raf = requestAnimationFrame(tick);
}

async function pickSub(s: Subtitle | null) {
  subPick.value = s;
  cue.value = "";
  cues = [];
  cancelAnimationFrame(raf);
  // the file chosen for this video, and the language to try first on the next one
  save(`subPick:${props.path}`, s?.url ?? null);
  save("subLang", s ? s.lang ?? "" : null);
  if (!s) return;
  subLoading.value = true;
  try {
    const vtt = await invoke<string>("video_subtitle", { url: s.url });
    if (subPick.value !== s) return;
    cues = parseVtt(vtt);
    tick();
  } catch (e) {
    error.value = `Subtitle tidak bisa dimuat (${e}).`;
    subPick.value = null;
  } finally {
    subLoading.value = false;
  }
}

// how subtitles look is remembered for every video, the delay per video; both in the app database
const SUB_STYLE = "neptunus.subStyle";
const delayKey = `subDelay:${props.path}`;
const subStyle = ref({ size: 1, color: "#ffffff", bg: 0.6, pos: 7 }); // pos: % from the bottom of the video
const subDelay = ref(0);
let prefsLoaded = false;
let saveTimer: ReturnType<typeof setTimeout> | undefined;
const save = (key: string, value: string | null) => invoke("setting_set", { key, value }).catch(() => {});
function flushPrefs() {
  clearTimeout(saveTimer);
  saveTimer = undefined;
  save("subStyle", JSON.stringify(subStyle.value));
  save(delayKey, subDelay.value ? String(subDelay.value) : null);
}
watch([subStyle, subDelay], () => {
  if (!prefsLoaded) return;
  clearTimeout(saveTimer); // sliders fire many times a second
  saveTimer = setTimeout(flushPrefs, 400);
}, { deep: true });
async function loadPrefs() {
  try {
    let style = await invoke<string | null>("setting_get", { key: "subStyle" });
    // older builds kept it in the webview's storage: carried over once
    if (!style) try { style = localStorage.getItem(SUB_STYLE); if (style) { save("subStyle", style); localStorage.removeItem(SUB_STYLE); } } catch {}
    if (style) subStyle.value = { ...subStyle.value, ...JSON.parse(style) };
    subDelay.value = Number(await invoke<string | null>("setting_get", { key: delayKey })) || 0;
  } catch {}
  prefsLoaded = true;
}
const nudge = (d: number) => (subDelay.value = Math.round((subDelay.value + d) * 10) / 10);
const SIZES = [{ v: 0.75, l: "Kecil" }, { v: 1, l: "Sedang" }, { v: 1.35, l: "Besar" }, { v: 1.75, l: "Sangat besar" }];
const COLORS = ["#ffffff", "#ffeb3b", "#7fdbff", "#b9f6ca", "#ffb3c1"];
const BGS = [{ v: 0, l: "Tanpa" }, { v: 0.4, l: "Tipis" }, { v: 0.6, l: "Sedang" }, { v: 0.85, l: "Pekat" }];
// a sample line while a style control moves, at the exact spot it will sit (no lift for the bar)
const preview = ref(false);
let previewTimer: ReturnType<typeof setTimeout> | undefined;
function showPreview() {
  preview.value = true;
  clearTimeout(previewTimer);
  previewTimer = setTimeout(() => (preview.value = false), 1500);
}
const cueText = computed(() => cue.value || (preview.value ? "Contoh subtitle\nseperti ini tampilannya" : ""));
const cueStyle = computed(() => ({
  "--pos": `${subStyle.value.pos}%`,
  fontSize: `calc(clamp(16px, 2.4vw, 30px) * ${subStyle.value.size})`,
  color: subStyle.value.color,
  background: `rgb(0 0 0 / ${subStyle.value.bg})`,
  textShadow: subStyle.value.bg < 0.3 ? "0 0 3px #000, 0 0 3px #000, 1px 1px 2px #000" : "0 1px 3px #000",
}));

async function loadSubs() {
  if (!props.sub?.sources.length) return;
  const [list, url, lang] = await Promise.all([
    subtitles(props.sub),
    invoke<string | null>("setting_get", { key: `subPick:${props.path}` }).catch(() => null),
    invoke<string | null>("setting_get", { key: "subLang" }).catch(() => null),
  ]);
  subs.value = list;
  const again = list.find((s) => s.url === url) ?? (lang !== null ? list.find((s) => (s.lang ?? "") === lang) : undefined);
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
  else if (e.key === "c" && (props.sub?.sources.length || audios.value.length > 1)) menu.value = !menu.value;
  else if ((e.key === "g" || e.key === "h") && subPick.value) nudge(e.key === "g" ? -0.1 : 0.1); // VLC's keys
  else return;
  wake();
}

onMounted(async () => {
  window.addEventListener("keydown", onKey);
  document.addEventListener("fullscreenchange", onFull);
  wake();
  loadPrefs();
  loadSubs();
  try {
    const url = (await invoke<string | null>("video_url", { path: props.path })) ?? convertFileSrc(props.path);
    // closed while the URL was on its way: stop the download it just started, play nothing
    if (closed) return void invoke("video_stop", { url }).catch(() => {});
    resolvedUrl = url;
    const v = video.value!;
    let start: number | null = null;
    // older plays were kept by stream URL
    try { start = Number(localStorage.getItem(posKey) ?? localStorage.getItem(`video-pos:${props.path}`)) || null; } catch {}
    if (start) v.addEventListener("loadedmetadata", () => { v.currentTime = start; }, { once: true });
    v.addEventListener("loadedmetadata", readAudios);
    (v as any).audioTracks?.addEventListener?.("addtrack", readAudios);
    const fail = (why: string) => {
      error.value = `Format video ini tidak didukung di perangkat ini. Coba sumber lain, misalnya yang H.264/x264. [${why}]`;
      buffering.value = false;
    };
    // the system player first: GStreamer (Linux), WebKit (Apple) and Android play HLS themselves and,
    // unlike hls.js, fetch segments without CORS. hls.js only steps in when that fails, which in
    // practice is WebView2 on Windows.
    v.addEventListener("error", async () => {
      if (closed) return;
      const native = `${v.error?.code ?? "?"}${v.error?.message ? `: ${v.error.message}` : ""}`;
      if (ext(props.path) === "m3u8" && !hls) {
        const Hls = (await import("hls.js")).default;
        if (Hls.isSupported()) {
          const h = new Hls({ maxBufferLength: 60, maxMaxBufferLength: 180, backBufferLength: 30, startPosition: start ?? -1 });
          hls = h;
          h.on(Hls.Events.AUDIO_TRACKS_UPDATED, readAudios);
          h.on(Hls.Events.AUDIO_TRACK_SWITCHED, readAudios);
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

// Closing must let go of the media pipeline: a <video> taken out of the page keeps its decoder, buffers
// and connection until garbage collection, which on phones can be much later, so each film opened and
// closed would add one more and the whole app would slow down.
onBeforeUnmount(() => {
  savePos(true); // while the element still knows where it was
  closed = true;
  hls?.destroy();
  hls = undefined;
  const v = video.value;
  if (v) {
    v.pause();
    v.removeAttribute("src");
    v.load();
  }
});

onUnmounted(() => {
  // iOS: the video took the audio session, and the music output it stopped has to be opened again
  if (!droid && document.documentElement.classList.contains("mobile")) invoke("audio_after_video").catch(() => {});
  window.removeEventListener("keydown", onKey);
  document.removeEventListener("fullscreenchange", onFull);
  clearTimeout(idleTimer);
  clearTimeout(previewTimer);
  clearTimeout(pictureTimer);
  cancelAnimationFrame(raf);
  if (saveTimer) flushPrefs();
  invoke("video_stop", { url: resolvedUrl }).catch(() => {});
  if (full.value) droid?.fullscreen?.(false);
});
</script>

<template>
  <div ref="box" class="vplayer" :class="{ idle, full }" role="dialog" :aria-label="title" @pointermove="wake" @pointerdown="down">
    <video ref="video" autoplay playsinline preload="auto" @click="tap" @dblclick="toggleFull"
      @webkitbeginfullscreen="nativeSubs(true)" @webkitendfullscreen="nativeSubs(false)"
      @timeupdate="sync(); savePos()" @progress="sync" @durationchange="sync" @pause="playing = false; idle = false; savePos(true)" @play="playing = true; wake()"
      @ended="savePos(true)" @volumechange="volume = video!.volume; muted = video!.muted"
      @waiting="buffering = true" @loadstart="buffering = true" @playing="buffering = false; checkPicture()" @canplay="buffering = false" @seeked="buffering = false"></video>
    <div v-if="buffering && !error" class="spin" aria-label="Memuat"></div>
    <p v-if="cueText" class="cue" :class="{ preview }" :style="cueStyle">{{ cueText }}</p>
    <p v-if="error" class="err">{{ error }}</p>

    <!-- subtitle panel: over the right of the video; a click anywhere else closes it -->
    <div v-if="menu" class="scrim" @pointerdown.stop="menu = false"></div>
    <aside v-if="menu" class="panel" role="dialog" :aria-label="panelName" @pointerdown.stop>
      <header>
        <strong>{{ panelName }}</strong>
        <button title="Tutup (Esc)" @click="menu = false"><Icon name="close" :size="22" /></button>
      </header>
      <!-- top: the language list scrolls on its own; bottom: delay and style stay in reach -->
      <section v-if="audios.length > 1" class="audios">
        <h4>Audio</h4>
        <button v-for="(a, i) in audios" :key="i" class="opt" :class="{ on: audioOn === i }" role="menuitemradio" :aria-checked="audioOn === i"
          :title="a" @click="pickAudio(i)"><Icon name="done" :size="18" /><span class="mlabel">{{ a }}</span></button>
      </section>
      <template v-if="sub?.sources.length">
      <section class="langs">
          <h4>Bahasa</h4>
          <button class="opt" :class="{ on: !subPick }" role="menuitemradio" :aria-checked="!subPick" @click="pickSub(null)"><Icon name="done" :size="18" />Mati</button>
          <p v-if="subs === null" class="mnote">Mencari subtitle…</p>
          <p v-else-if="!subs.length" class="mnote">Tidak ada subtitle untuk judul ini.</p>
          <button v-for="x in subList" :key="x.s.url" class="opt" :class="{ on: subPick === x.s }" role="menuitemradio" :aria-checked="subPick === x.s"
            :title="x.label" @click="pickSub(x.s)">
            <Icon name="done" :size="18" /><span class="mlabel">{{ x.label }}</span><small v-if="subPick === x.s && subLoading">memuat…</small>
          </button>
      </section>
      <div class="settings">
        <section>
          <h4>Delay <small>g / h</small></h4>
          <div class="delay">
            <button title="Lebih cepat 0,1 detik (g)" @click="nudge(-0.1)">−</button>
            <output>{{ subDelay > 0 ? "+" : "" }}{{ subDelay.toFixed(1) }} detik</output>
            <button title="Lebih lambat 0,1 detik (h)" @click="nudge(0.1)">+</button>
            <button class="reset" :disabled="!subDelay" @click="subDelay = 0">Reset</button>
          </div>
          <input type="range" min="-10" max="10" step="0.1" v-model.number="subDelay" aria-label="Delay subtitle"
            :style="{ '--done': `${(subDelay + 10) * 5}%` }" />
        </section>
        <section>
          <h4>Tampilan</h4>
          <div class="srow"><span class="lbl">Ukuran</span>
            <div class="chips"><button v-for="z in SIZES" :key="z.v" :class="{ on: subStyle.size === z.v }" @click="subStyle.size = z.v; showPreview()">{{ z.l }}</button></div></div>
          <div class="srow"><span class="lbl">Warna</span>
            <div class="chips">
              <button v-for="c in COLORS" :key="c" class="swatch" :class="{ on: subStyle.color === c }" :style="{ background: c }" :title="c" @click="subStyle.color = c; showPreview()"></button>
            </div></div>
          <div class="srow"><span class="lbl">Posisi</span>
            <input type="range" min="0" max="85" step="0.5" v-model.number="subStyle.pos" aria-label="Posisi subtitle dari bawah" @input="showPreview"
              :style="{ '--done': `${(subStyle.pos / 85) * 100}%` }" /></div>
          <div class="srow"><span class="lbl">Latar</span>
            <div class="chips"><button v-for="b in BGS" :key="b.v" :class="{ on: subStyle.bg === b.v }" @click="subStyle.bg = b.v; showPreview()">{{ b.l }}</button></div></div>
        </section>
      </div>
      </template>
    </aside>

    <div class="bar top">
      <button title="Tutup (Esc)" @click="emit('close')"><Icon name="back" /></button>
      <strong>{{ title }}</strong>
    </div>

    <div v-if="flash" :key="flash.n" class="flash"><Icon :name="flash.icon" :size="40" /></div>

    <div class="bar bottom" @pointerdown.stop>
      <div ref="seekEl" class="seek" role="slider" tabindex="0" aria-label="Posisi" :aria-valuemax="Math.round(duration)" :aria-valuenow="Math.round(time)"
        :aria-valuetext="fmt(time)" :style="{ '--done': pct(time), '--buf': pct(buffered), '--hover': hover === null ? '0%' : `${hover * 100}%` }"
        @pointerdown="seekDown" @pointermove="seekMove" @pointerup="dragging = false" @pointercancel="dragging = false" @pointerleave="hover = null" @keydown="seekKey">
        <div class="track"><i class="buf"></i><i class="hov"></i><i class="done"></i></div>
        <span class="thumb"></span>
        <span v-if="hover !== null && duration" class="tip">{{ fmt(hover * duration) }}</span>
      </div>
      <div class="row">
        <button :title="playing ? 'Jeda (k)' : 'Putar (k)'" @click="toggle"><Icon :name="playing ? 'pause' : 'play'" :size="32" /></button>
        <button title="Mundur 10 detik (←)" @click="skip(-10)"><Icon name="replay10" :size="26" /></button>
        <button title="Maju 10 detik (→)" @click="skip(10)"><Icon name="forward10" :size="26" /></button>
        <div class="vol">
          <button :title="muted ? 'Bunyikan (m)' : 'Bisukan (m)'" @click="toggleMute"><Icon :name="muted || !volume ? 'volumeOff' : 'volume'" :size="26" /></button>
          <input type="range" min="0" max="1" step="0.05" :value="muted ? 0 : volume" aria-label="Volume" :style="{ '--done': `${(muted ? 0 : volume) * 100}%` }" @input="setVolume" />
        </div>
        <span class="time">{{ fmt(time) }} / {{ fmt(duration) }}</span>
        <span class="grow"></span>
        <button v-if="sub?.sources.length || audios.length > 1" class="cc" :class="{ on: subPick }" :title="`${panelName} (c)`"
          @click="menu = !menu"><Icon name="subtitles" :size="26" /></button>
        <button :title="full ? 'Keluar layar penuh (f)' : 'Layar penuh (f)'" @click="toggleFull"><Icon :name="full ? 'fullscreenExit' : 'fullscreen'" :size="30" /></button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.vplayer {
  position: fixed; inset: var(--titlebar) 0 0; z-index: 30; background: #000; color: #fff;
  display: flex; align-items: center; justify-content: center;
}
.vplayer:fullscreen, .vplayer.full { inset: 0; z-index: 32; }
.vplayer.idle { cursor: none; }
/* pinned to the box: a grid row sized by the video itself let tall frames spill under the window edge */
video { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; background: #000; }

.bar {
  position: absolute; left: 0; right: 0; display: flex; gap: 4px;
  padding-left: max(12px, env(safe-area-inset-left)); padding-right: max(12px, env(safe-area-inset-right));
  transition: opacity 0.25s;
}
.idle .bar { opacity: 0; pointer-events: none; }
.top { top: 0; align-items: center; padding-top: 8px; padding-bottom: 40px; background: linear-gradient(rgb(0 0 0 / 0.65), transparent); }
.top strong { flex: 1; font-size: 18px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-shadow: 0 0 2px rgb(0 0 0 / 0.5); }
.bottom {
  bottom: 0; flex-direction: column; gap: 0; padding-top: 60px; padding-bottom: max(2px, env(safe-area-inset-bottom));
  background: linear-gradient(transparent, rgb(0 0 0 / 0.75));
}
.row { display: flex; align-items: center; height: 48px; }
.grow { flex: 1; }
.bar button {
  position: relative; color: inherit; width: 48px; height: 48px; display: inline-grid; place-items: center; flex: none;
  opacity: 0.9; transition: opacity 0.1s;
}
.bar button:hover { opacity: 1; }
.top button { width: 44px; height: 44px; border-radius: 22px; }
.top button:hover { background: rgb(255 255 255 / 0.12); }
.time { font-size: 14px; font-variant-numeric: tabular-nums; margin-left: 8px; white-space: nowrap; opacity: 0.95; text-shadow: 0 0 2px rgb(0 0 0 / 0.5); }
/* the red underline YouTube puts under CC when captions are on */
.cc::after {
  content: ""; position: absolute; left: 12px; right: 12px; bottom: 9px; height: 3px; border-radius: 2px; background: #f03;
  transform: scaleX(0); transition: transform 0.2s;
}
.cc.on::after { transform: scaleX(1); }

/* seek: thin line that thickens on hover; buffered, hovered and played layered over it */
.seek { position: relative; height: 16px; margin: 0 2px; display: flex; align-items: center; cursor: pointer; touch-action: none; outline: none; }
.track { position: relative; width: 100%; height: 3px; background: rgb(255 255 255 / 0.2); transition: height 0.1s; }
.seek:hover .track, .seek:focus-visible .track { height: 5px; }
.track i { position: absolute; inset: 0 auto 0 0; }
.buf { width: var(--buf); background: rgb(255 255 255 / 0.4); }
.hov { width: var(--hover); background: rgb(255 255 255 / 0.5); }
.done { width: var(--done); background: #f03; }
.thumb {
  position: absolute; left: var(--done); width: 13px; height: 13px; margin-left: -6.5px; border-radius: 50%; background: #f03;
  transform: scale(0); transition: transform 0.1s;
}
.seek:hover .thumb, .seek:focus-visible .thumb { transform: scale(1); }
.tip {
  position: absolute; bottom: 22px; left: clamp(24px, var(--hover), calc(100% - 24px)); transform: translateX(-50%);
  padding: 3px 8px; border-radius: 4px; background: rgb(28 28 28 / 0.9); font-size: 13px; font-weight: 500;
  font-variant-numeric: tabular-nums; pointer-events: none; white-space: nowrap;
}
input[type=range] { -webkit-appearance: none; appearance: none; background: none; margin: 0; cursor: pointer; }
input::-webkit-slider-thumb { -webkit-appearance: none; width: 12px; height: 12px; border-radius: 50%; background: #fff; margin-top: -4.5px; }
input::-moz-range-thumb { width: 12px; height: 12px; border: 0; border-radius: 50%; background: #fff; }
.vol { display: flex; align-items: center; }
.vol input { width: 0; height: 20px; opacity: 0; transition: width 0.2s, opacity 0.2s, margin 0.2s; }
.vol:hover input, .vol:focus-within input { width: 60px; opacity: 1; margin-right: 8px; }
.vol input::-webkit-slider-runnable-track { height: 3px; background: linear-gradient(to right, #fff var(--done), rgb(255 255 255 / 0.25) var(--done)); }
.vol input::-moz-range-track { height: 3px; background: linear-gradient(to right, #fff var(--done), rgb(255 255 255 / 0.25) var(--done)); }

.flash {
  position: absolute; width: 72px; height: 72px; border-radius: 50%; display: grid; place-items: center; pointer-events: none;
  background: rgb(0 0 0 / 0.5); animation: flash 0.6s ease-out forwards;
}
@keyframes flash { from { opacity: 1; transform: scale(1); } to { opacity: 0; transform: scale(1.6); } }
.scrim { position: absolute; inset: 0; z-index: 2; }
.panel {
  position: absolute; z-index: 3; top: 12px; right: 12px; bottom: 76px; width: min(340px, calc(100% - 24px));
  display: flex; flex-direction: column; border-radius: 16px; overflow: hidden;
  background: rgb(20 20 20 / 0.72); backdrop-filter: blur(18px) saturate(1.3); box-shadow: 0 8px 40px rgb(0 0 0 / 0.45);
  animation: slide 0.25s var(--spring);
}
@keyframes slide { from { opacity: 0; transform: translateX(24px); } }
.panel header { display: flex; align-items: center; justify-content: space-between; padding: 8px 8px 4px 18px; }
.panel header strong { font-size: 16px; font-weight: 600; }
.panel header button { width: 40px; height: 40px; border-radius: 20px; display: grid; place-items: center; color: inherit; }
.panel header button:hover { background: rgb(255 255 255 / 0.12); }
.panel section { padding: 8px 10px; display: flex; flex-direction: column; gap: 6px; }
.audios { flex: none; max-height: 35%; overflow-y: auto; scrollbar-width: thin; }
.langs { flex: 1; min-height: 120px; overflow-y: auto; scrollbar-width: thin; }
.settings { flex: none; border-top: 1px solid rgb(255 255 255 / 0.12); background: rgb(0 0 0 / 0.25); padding-bottom: 6px; }
.settings section + section { border-top: 1px solid rgb(255 255 255 / 0.08); }
.srow { display: flex; align-items: center; gap: 8px; }
.srow .lbl { flex: none; width: 52px; margin: 0 0 0 8px; }
.srow .chips { padding: 0; gap: 4px; }
.panel h4 { margin: 2px 8px 2px; font-size: 12px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; opacity: 0.6; }
.panel h4 small { margin-left: 6px; font-weight: 500; text-transform: none; letter-spacing: 0; opacity: 0.8; }
.mnote { margin: 2px 8px; font-size: 13px; opacity: 0.65; }
.opt { width: 100%; min-height: 40px; border-radius: 10px; display: flex; align-items: center; gap: 10px; padding: 0 10px; font-size: 14px; text-align: left; color: inherit; }
.opt:hover { background: rgb(255 255 255 / 0.1); }
.opt.on { background: rgb(255 255 255 / 0.14); font-weight: 600; }
.opt :deep(svg) { flex: none; opacity: 0; }
.opt.on :deep(svg) { opacity: 1; color: #f03; }
.opt small { margin-left: auto; opacity: 0.6; }
.mlabel { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.delay { display: flex; align-items: center; gap: 6px; padding: 0 4px; }
.delay button {
  min-width: 40px; height: 36px; border-radius: 18px; font-size: 20px; color: inherit; background: rgb(255 255 255 / 0.1);
}
.delay button:hover:not(:disabled) { background: rgb(255 255 255 / 0.2); }
.delay output { flex: 1; text-align: center; font-size: 15px; font-weight: 600; font-variant-numeric: tabular-nums; }
.delay .reset { font-size: 13px; padding: 0 12px; }
.delay .reset:disabled { opacity: 0.35; }
.panel input[type=range] { width: calc(100% - 16px); margin: 4px 8px; height: 20px; }
.srow input[type=range] { flex: 1; width: auto; margin: 0 8px 0 0; }
.panel input::-webkit-slider-runnable-track { height: 4px; border-radius: 2px; background: linear-gradient(to right, #f03 var(--done), rgb(255 255 255 / 0.25) var(--done)); }
.panel input::-moz-range-track { height: 4px; border-radius: 2px; background: rgb(255 255 255 / 0.25); }
.panel input::-webkit-slider-thumb { margin-top: -4px; }
.lbl { margin: 4px 8px 0; font-size: 13px; opacity: 0.75; }
.chips { display: flex; flex-wrap: wrap; gap: 6px; padding: 0 6px; }
.chips button {
  height: 28px; padding: 0 10px; border-radius: 14px; font-size: 13px; color: inherit;
  background: rgb(255 255 255 / 0.1); border: 2px solid transparent;
}
.chips button:hover { background: rgb(255 255 255 / 0.18); }
.chips button.on { background: #fff; color: #000; font-weight: 600; }
.chips .swatch { width: 28px; padding: 0; border-color: rgb(255 255 255 / 0.25); }
.chips .swatch.on { border-color: #f03; box-shadow: 0 0 0 2px rgb(20 20 20); }
.cue {
  position: absolute; left: 50%; bottom: var(--pos, 7%); transform: translateX(-50%); max-width: min(90%, 1000px); margin: 0;
  padding: 4px 12px; border-radius: 8px; white-space: pre-line; text-align: center;
  line-height: 1.35; font-weight: 500; pointer-events: none;
  transition: bottom 0.35s var(--spring);
}
/* lifted clear of the control bar while it shows, wherever it was placed */
.vplayer:not(.idle) .cue:not(.preview) { bottom: max(var(--pos, 7%), 100px); }
.cue.preview { z-index: 4; transition: none; }
.spin {
  position: absolute; width: 56px; height: 56px; border-radius: 50%; pointer-events: none;
  border: 5px solid rgb(255 255 255 / 0.25); border-top-color: #fff; animation: spin 0.9s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }
.err { position: absolute; background: rgb(0 0 0 / 0.75); padding: 12px 20px; border-radius: 16px; margin: 0 16px; text-align: center; }
@media (max-width: 600px) {
  .bar button { width: 40px; height: 40px; }
  .vol input { display: none; }
  .time { font-size: 13px; margin-left: 4px; }
}
</style>
