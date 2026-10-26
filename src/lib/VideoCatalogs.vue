<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref, toRaw, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import Icon from "./Icon.vue";
import CatalogRow from "./CatalogRow.vue";
import Dropdown from "./Dropdown.vue";
import { extName, type ExtInfo } from "./ext";
import { featureOn, loadFeatures } from "./videoprefs";
import { cached } from "./vcache";
import { poster } from "./posters";
import { clock, forget, lastPlayed, progressOf, recordPlay, resumeKey, toggleBookmark, vlib, type Saved, type Watched } from "./vlibrary";
import type { RowItem } from "./CatalogRow.vue";
import {
  allStreams, subtitleQuery, CINEMETA, meta, rowCatalogs, searchCatalog, sourceInfo, videoSources,
  type Episode, type Meta, type SourceInfo, type SourceStream, type StreamGroup, type SubtitleQuery,
} from "./videoext";

// Video › Ekstensi: every addon with catalogs stacked as its own section, each catalog a
// horizontal row. Playing asks all addons for streams, not just the one the title came from.
const props = defineProps<{ exts: ExtInfo[]; query: string; home?: number; reload?: number }>();
const emit = defineEmits<{ play: [v: { path: string; title: string; sub?: SubtitleQuery; resume?: string; cached?: string[]; headers?: Record<string, string> }]; changed: [ExtInfo[]]; detail: [open: boolean]; refreshing: [busy: boolean] }>();

const CINEMETA_KEY = "neptunus.cinemetaAdded";
const infos = reactive<Record<string, SourceInfo | "error">>({});
const isCinemeta = (e: ExtInfo) => String(e.values?.manifestUrl ?? "").includes("cinemeta");
const sources = computed(() =>
  videoSources(props.exts).sort((a, b) => Number(isCinemeta(b)) - Number(isCinemeta(a))));
const ready = (id: string) => { const i = infos[id]; return i && i !== "error" ? i : undefined; };
loadFeatures();
const sections = computed(() =>
  sources.value.filter((e) => featureOn(e.id, "catalog"))
    .map((e) => ({ e, info: ready(e.id) })).filter((s) => !s.info || rowCatalogs(s.info).length));
const kindName = (t: string) => ({ movie: "Film", series: "Serial", anime: "Anime", tv: "TV", channel: "Kanal" })[t] ?? t;

// header search, debounced: each addon's search catalogs become rows of results
const q = ref("");
let debounce: ReturnType<typeof setTimeout>;
watch(() => props.query, (v) => { clearTimeout(debounce); debounce = setTimeout(() => (q.value = v.trim()), 450); });
const searches = computed(() =>
  sources.value.flatMap((e) => {
    const info = ready(e.id);
    if (!info || !featureOn(e.id, "catalog")) return [];
    return ["movie", "series"].flatMap((t) => {
      const c = searchCatalog(info.catalogs, t);
      return c && (c.id.toLowerCase().includes("search") || (c.extra ?? []).some((x) => x.name === "search") || c.extraSupported?.includes("search"))
        ? [{ e, c, title: `${extName(e)} · ${kindName(t)}` }] : [];
    });
  }));

watch(sources, (list) => {
  for (const e of list) {
    if (e.id in infos) continue;
    infos[e.id] = undefined as never;
    sourceInfo(e.id).then((i) => (infos[e.id] = i), () => (infos[e.id] = "error"));
  }
}, { immediate: true });

// "Muat ulang": every addon's manifest and catalogs asked again, the database updated with the answers
const refreshN = ref(0);
const refreshing = ref(false);
async function refreshAll() {
  refreshing.value = true;
  await Promise.all(sources.value.map((e) => sourceInfo(e.id, true).then((i) => (infos[e.id] = i), () => {})));
  refreshN.value++;
  refreshing.value = false;
}

// Cinemeta is the default catalog: added once; removing it in settings keeps it removed
onMounted(async () => {
  let added = false;
  try { added = !!localStorage.getItem(CINEMETA_KEY); } catch {}
  if (added || props.exts.some(isCinemeta)) return;
  try {
    await invoke("ext_install_stremio", { url: CINEMETA });
    try { localStorage.setItem(CINEMETA_KEY, "1"); } catch {}
    emit("changed", await invoke<ExtInfo[]>("ext_list"));
  } catch {}
});

// --- detail page: episodes (series) and the stream picker ---
const open = ref<{ m: Meta; source: string } | null>(null);
const detail = ref<Meta | null>(null);
const episode = ref<Episode | null>(null);
const found = ref<StreamGroup[] | null>(null);
const sheetError = ref("");
let ask = 0;

const seasons = computed(() => {
  const by = new Map<number, Episode[]>();
  for (const ep of detail.value?.videos ?? []) {
    const s = ep.season ?? 1;
    if (!by.has(s)) by.set(s, []);
    by.get(s)!.push(ep);
  }
  return [...by.entries()].sort(([a], [b]) => (a || 999) - (b || 999));
});
const season = ref(1);

let scrollBack = 0;
const scroller = () => document.querySelector("main");

// the open title survives a reload, like the page around it
const DETAIL_KEY = "neptunus.videoDetail";
watch(open, (o) => { try { o ? sessionStorage.setItem(DETAIL_KEY, JSON.stringify(o)) : sessionStorage.removeItem(DETAIL_KEY); } catch {} });
// restored once the extension list is in, or the picker would find no addon to ask
let restored = false;
watch(sources, (list) => {
  if (restored || !list.length) return;
  restored = true;
  try {
    const o = JSON.parse(sessionStorage.getItem(DETAIL_KEY) ?? "null");
    if (o?.m && o.source && !open.value) openMeta(o.m, o.source);
  } catch {}
}, { immediate: true });

// Some catalog addons list a series but give no episodes for it. Ask the other addons (Cinemeta has
// most IMDb titles), then TVmaze, which also knows the newest anime. Episode ids follow Stremio's
// "<imdb>:<season>:<episode>" when there is an IMDb id, so stream addons still find them.
type TvEp = { season: number; number: number; name?: string; airdate?: string; image?: { medium?: string } };
const tvEpisodes = (show: { _embedded?: { episodes?: TvEp[] } }, base: string) =>
  (show._embedded?.episodes ?? []).map((e) => ({
    id: `${base}:${e.season}:${e.number}`, name: e.name, season: e.season, episode: e.number, released: e.airdate, thumbnail: e.image?.medium,
  }));
const tvmaze = (url: string) => fetch(`https://api.tvmaze.com/${url}`).then((r) => (r.ok ? r.json() : null)).catch(() => null);

async function episodesElsewhere(m: Meta, source: string): Promise<Partial<Meta> | null> {
  let id = m.id;
  let show = id.startsWith("tvmaze:") ? await tvmaze(`shows/${encodeURIComponent(id.slice(7))}?embed=episodes`) : null;
  if (show?.externals?.imdb) id = show.externals.imdb;
  for (const e of id.startsWith("tt") ? sources.value : []) {
    if (e.id === source && id === m.id) continue;
    try {
      const full = await meta(e.id, "series", id);
      if (full?.videos?.length) return { id, videos: full.videos };
    } catch {}
  }
  if (!show && id.startsWith("tt")) {
    const found = await tvmaze(`lookup/shows?imdb=${encodeURIComponent(id)}`);
    if (found?.id) show = await tvmaze(`shows/${found.id}?embed=episodes`);
  }
  const videos = show ? tvEpisodes(show, id.startsWith("tt") ? id : m.id) : [];
  return videos.length ? { id, videos } : null;
}

// whether `m` is still the open title (a newer one may have been opened while its details loaded);
// `open` keeps a reactive copy, so compare the objects underneath
const isOpen = (m: Meta) => toRaw(open.value?.m) === toRaw(m);

async function openMeta(m: Meta, source: string) {
  scrollBack = scroller()?.scrollTop ?? 0;
  open.value = { m, source };
  requestAnimationFrame(() => scroller()?.scrollTo({ top: 0 }));
  detail.value = m;
  episode.value = null;
  found.value = null;
  sheetError.value = "";
  try {
    const full = await meta(source, m.type, m.id);
    if (isOpen(m) && full) detail.value = { ...m, ...full };
  } catch {}
  if (m.type === "series" && !detail.value?.videos?.length) {
    const more = await episodesElsewhere(m, source);
    if (isOpen(m) && more) detail.value = { ...detail.value!, ...more };
  }
  if (m.type === "series") season.value = seasons.value.find(([s]) => s > 0)?.[0] ?? seasons.value[0]?.[0] ?? 1;
  else findStreams(m.type, detail.value?.id ?? m.id);
}

let lastFind: [string, string] = ["", ""];
// addons still being asked for the open title's sources
const pending = ref(0);
async function findStreams(type: string, id: string, force = false) {
  lastFind = [type, id];
  const mine = ++ask;
  found.value = null;
  sheetError.value = "";
  const all = sources.value.filter((e) => featureOn(e.id, "streams")).map((e) => ({ id: e.id, name: extName(e), info: ready(e.id) }));
  pending.value = all.length;
  // each addon's sources show as soon as it answers, in the order the addon gives them; the tabs keep
  // the order the addons are installed in (not which answered first), with the ones that had nothing last
  const order = (g: StreamGroup) => (g.list.length ? 0 : all.length) + all.findIndex((e) => e.id === g.id);
  const list = await allStreams(all, type, id, force, (g) => {
    if (mine !== ask) return;
    pending.value--;
    found.value = [...(found.value ?? []), g].sort((a, b) => order(a) - order(b));
  });
  if (mine !== ask) return;
  pending.value = 0;
  if (!list.length) {
    found.value = [];
    sheetError.value = "Belum ada ekstensi yang menyediakan sumber video.";
  }
}

// the picker groups streams by the addon they came from, one tab each
const pickTab = ref("");
const groups = computed(() => found.value ?? []);
const tab = computed(() => groups.value.find((g) => g.id === pickTab.value) ?? groups.value[0]);
const tabList = computed(() => tab.value?.list ?? []);
// each source's badges and lines worked out once, not on every redraw of every row
const tabRows = computed(() => tabList.value.map((s) => ({ s, d: describe(s) })));
watch(groups, (g) => { if (!g.some((x) => x.id === pickTab.value)) pickTab.value = g[0]?.id ?? ""; });

// A stream's name/title are free text ("1080p", "💾 1.4 GB", file names…): pull out quality, size and
// the usual release tags for badges, and keep the rest as a headline plus detail lines.
const TAGS: [RegExp, string][] = [
  [/\b(dolby ?vision|dovi|\bdv\b)/i, "Dolby Vision"], [/\bhdr10\+/i, "HDR10+"], [/\bhdr(10)?\b/i, "HDR"],
  [/\b(hevc|x\.?265|h\.?265)\b/i, "HEVC"], [/\b(avc|x\.?264|h\.?264)\b/i, "H.264"], [/\bav1\b/i, "AV1"],
  [/\bremux\b/i, "Remux"], [/\b(blu-?ray|bdrip|brrip)\b/i, "BluRay"], [/\bweb-?dl\b/i, "WEB-DL"], [/\bweb-?rip\b/i, "WEBRip"],
  [/\bhdtv\b/i, "HDTV"], [/\b(atmos)\b/i, "Atmos"], [/\b(ddp|dd\+|eac3)\s?(5\.1|7\.1)?/i, "DD+"], [/\b(dts(-hd)?)\b/i, "DTS"],
  [/\b(7\.1|5\.1)\b/, "$1"], [/\baac\b/i, "AAC"], [/\b(sub(title)?s?|subtitle|indo ?sub)\b/i, "Subtitle"],
  [/\|\^\||[.\s_-]pad([.\s_-]|$)/i, "padded · sering gagal"],
];
const bytes = (n: number) => (n >= 1 << 30 ? `${(n / (1 << 30)).toFixed(2)} GB` : `${Math.round(n / (1 << 20))} MB`);
function describe(s: SourceStream) {
  const file = s.behaviorHints?.filename ?? "";
  const text = [s.name, s.title, file].filter(Boolean).join("\n");
  const q = text.match(/\b(2160p|4k|uhd|1440p|1080p|720p|576p|480p|360p)\b/i)?.[1];
  const quality = q ? (/^(4k|uhd|2160p)$/i.test(q) ? "4K" : q.toLowerCase()) : "";
  const size = s.behaviorHints?.videoSize ? bytes(s.behaviorHints.videoSize)
    : text.match(/\d+(?:[.,]\d+)?\s?(?:GiB|MiB|GB|MB)\b/i)?.[0].replace(/i(?=B)/i, "") ?? "";
  const tags = [...new Set(TAGS.flatMap(([re, label]) => { const m = text.match(re); return m ? [label.replace("$1", m[1] ?? "")] : []; }))];
  // drop decoration-only lines and the ones that just repeat the badges
  const lines = [s.name, s.title].filter(Boolean).join("\n").split("\n").map((l) => l.trim()).filter((l) => l && !/^[\p{Extended_Pictographic}\s|·•-]+$/u.test(l));
  return { quality, size, tags, head: lines[0] ?? "Sumber", rest: lines.slice(1).join(" · "), file };
}

// episodes without a picture, or whose picture fails to load, show the series art with an icon
const badThumb = reactive(new Set<string>());
const seasonEps = computed(() => seasons.value.find(([s]) => s === season.value)?.[1] ?? []);
function setSeason(s: number) {
  season.value = s;
  episode.value = null;
  ask++; // drops a source search still running for the previous episode
}

// the episode strip scrolls by touch; with a mouse there are arrows, dimmed at either end
const epStrip = ref<HTMLElement>();
const epEnds = ref({ start: true, end: true });
function epScrolled() {
  const s = epStrip.value;
  // a few pixels of slack: the strip's padding and scroll snapping keep it just off 0 and the very end
  if (s) epEnds.value = { start: s.scrollLeft <= 6, end: s.scrollLeft + s.clientWidth >= s.scrollWidth - 6 };
}
const scrollEps = (dir: number) => epStrip.value?.scrollBy({ left: dir * epStrip.value.clientWidth * 0.8, behavior: "smooth" });
watch(seasonEps, () => requestAnimationFrame(epScrolled), { flush: "post" });

function pickEpisode(ep: Episode) {
  episode.value = ep;
  findStreams("series", ep.id);
}

const subSources = () => sources.value.filter((e) => featureOn(e.id, "subtitles")).map((e) => ({ id: e.id, name: extName(e), info: ready(e.id) }));

function play(s: SourceStream) {
  const m = detail.value!;
  const ep = episode.value;
  const title = ep ? `${m.name} · S${ep.season ?? 1}E${ep.episode ?? ""}${ep.name && !/^S\d/.test(ep.name) ? ` — ${ep.name}` : ""}` : m.name;
  // kept under the id the title was opened with, so the catalog row and the bookmark find it again
  const opened = { ...m, id: open.value!.m.id };
  recordPlay(opened, open.value!.source, ep, s, title);
  // marked as cached only once it really plays (VideoLibrary, on the player's `played`)
  emit("play", { path: s.url!, title, sub: subtitleQuery(subSources(), m.type, ep?.id ?? m.id, s), resume: ep?.id ?? opened.id,
    cached: ep ? [m.id, ep.id] : [m.id], headers: s.behaviorHints?.proxyHeaders?.request });
}

// --- the viewer's own rows: watched last and bookmarked ---
function noteOf(h: Watched) {
  const p = progressOf(resumeKey(h));
  const ep = h.episode ? `S${h.episode.season ?? 1}E${h.episode.episode ?? "?"}` : "";
  const at = p ? (p.dur ? `sisa ${clock(Math.max(0, p.dur - p.pos))}` : `dari ${clock(p.pos)}`) : "";
  return [ep, at].filter(Boolean).join(" · ") || "Sudah ditonton";
}
const watching = computed<RowItem[]>(() => {
  void vlib.seen; // read again after a player closes
  return vlib.history.map((h) => {
    const p = progressOf(resumeKey(h));
    return { ...h, note: noteOf(h), progress: p?.dur ? Math.min(1, p.pos / p.dur) : undefined };
  });
});
const bookmarked = computed<RowItem[]>(() => vlib.bookmarks);

/** A title from the viewer's rows: its page, with the episode watched last picked again. */
async function openSaved(m: Saved | Watched) {
  await openMeta(m, m.source);
  const ep = "episode" in m ? m.episode : undefined;
  if (!ep || !isOpen(m)) return;
  const full = detail.value?.videos?.find((e) => e.id === ep.id) ?? ep;
  season.value = full.season ?? season.value;
  pickEpisode(full);
}

// the page's backdrop: the title's background, else its poster blurred (from the poster cache)
const backdrop = computed(() => {
  const d = detail.value;
  return d && open.value ? poster(d.background || d.poster, open.value.m, d.background ? "backdrop" : "poster") : undefined;
});

// the open title's place in the viewer's rows
const openId = computed(() => open.value?.m.id ?? "");
const saved = computed(() => !!openId.value && vlib.bookmarks.some((b) => b.id === openId.value));
const resumable = computed(() => (openId.value ? lastPlayed(openId.value) : undefined));
const resumeLabel = computed(() => {
  void vlib.seen;
  const h = resumable.value;
  if (!h) return "";
  const p = progressOf(resumeKey(h));
  const ep = h.episode ? `S${h.episode.season ?? 1}E${h.episode.episode ?? "?"} ` : "";
  return `Lanjutkan ${ep}${p ? `· ${clock(p.pos)}` : ""}`.trim();
});
function bookmark() {
  if (open.value) toggleBookmark({ ...detail.value!, id: open.value.m.id }, open.value.source);
}
/** Plays the source watched last again, from where it was left. */
function resume() {
  const h = resumable.value;
  if (!h?.stream.url) return;
  recordPlay({ ...detail.value!, id: h.id }, h.source, h.episode ?? null, h.stream, h.title);
  emit("play", { path: h.stream.url, title: h.title, sub: subtitleQuery(subSources(), h.type, h.episode?.id ?? h.id, h.stream), resume: resumeKey(h),
    cached: h.episode ? [h.id, h.episode.id] : [h.id], headers: h.stream.behaviorHints?.proxyHeaders?.request });
}

watch(() => props.home, () => open.value && close());
// "Muat ulang katalog" also sits in the tab bar, in VideoLibrary
watch(() => props.reload, () => refreshAll());
watch(refreshing, (b) => emit("refreshing", b));
// the back button lives beside the tabs, in VideoLibrary
watch(open, (o) => emit("detail", !!o), { immediate: true });
// leaving the tab or the Video menu forgets the open title; only a reload brings it back
onUnmounted(() => { try { sessionStorage.removeItem(DETAIL_KEY); } catch {} });

function close() {
  open.value = null;
  ask++;
  requestAnimationFrame(() => scroller()?.scrollTo({ top: scrollBack }));
}
</script>

<template>
  <p v-if="!sources.length" class="none">Belum ada ekstensi video. Tambah addon Stremio di Pengaturan › Ekstensi › Video.</p>

  <div v-else-if="open && detail" class="page" :aria-label="detail.name">
    <div v-if="detail.background || detail.poster" class="backdrop" :class="{ blur: !detail.background }"
      :style="{ backgroundImage: `url(&quot;${backdrop}&quot;)` }"></div>
    <div class="hero">
      <img v-if="detail.poster" :src="poster(detail.poster, open!.m)" alt="" class="hposter" />
      <div class="htext">
        <h2>{{ detail.name }}</h2>
        <span class="sub">{{ [detail.releaseInfo || detail.year, kindName(detail.type)].filter(Boolean).join(" · ") }}</span>
        <p v-if="detail.description" class="desc">{{ detail.description }}</p>
        <div class="hacts">
          <button v-if="resumable" class="hbtn filled" @click="resume"><Icon name="play" :size="20" />{{ resumeLabel }}</button>
          <button class="hbtn" :class="{ on: saved }" :aria-pressed="saved" @click="bookmark">
            <Icon :name="saved ? 'bookmarkOn' : 'bookmark'" :size="20" />{{ saved ? "Tersimpan" : "Simpan" }}
          </button>
        </div>
      </div>
    </div>
    <template v-if="detail.type === 'series'">
      <p v-if="!seasons.length" class="none">Daftar episode tidak tersedia.</p>
      <div v-else class="epsel">
        <Dropdown class="sel" label="Musim" :model-value="season" @update:model-value="setSeason"
          :options="seasons.map(([s, eps]) => ({ value: s, label: `${s ? `Musim ${s}` : 'Spesial'} (${eps.length})` }))" />
        <div v-if="seasonEps.length" class="epnav">
          <button title="Episode sebelumnya" aria-label="Geser episode ke kiri" :disabled="epEnds.start" @click="scrollEps(-1)"><Icon name="back" :size="20" /></button>
          <button title="Episode berikutnya" aria-label="Geser episode ke kanan" class="fwd" :disabled="epEnds.end" @click="scrollEps(1)"><Icon name="back" :size="20" /></button>
        </div>
      </div>
      <div v-if="seasonEps.length" ref="epStrip" class="eps" role="listbox" aria-label="Episode" @scroll.passive="epScrolled">
        <button v-for="ep in seasonEps" :key="ep.id" class="ep" role="option" :aria-selected="episode?.id === ep.id"
          :class="{ on: episode?.id === ep.id, cached: cached.ids.has(ep.id) }" @click="pickEpisode(ep)">
          <div class="ethumb">
            <img v-if="ep.thumbnail && !badThumb.has(ep.id)" :src="poster(ep.thumbnail, ep, 'thumb')" loading="lazy" alt="" @error="badThumb.add(ep.id)" />
            <span v-else class="edefault" :style="detail.background || detail.poster ? { backgroundImage: `url(&quot;${backdrop}&quot;)` } : undefined">
              <Icon name="movie" :size="30" />
            </span>
            <span class="enum">E{{ ep.episode ?? "?" }}</span>
            <span v-if="cached.ids.has(ep.id)" class="saved" title="Sudah ada di cache"><Icon name="downloadDone" :size="14" /></span>
            <span v-if="episode?.id === ep.id" class="eplay"><Icon name="play" :size="26" /></span>
          </div>
          <strong>{{ ep.name || `Episode ${ep.episode ?? ""}` }}</strong>
          <span v-if="ep.released">{{ new Date(ep.released).toLocaleDateString("id-ID", { dateStyle: "medium" }) }}</span>
        </button>
      </div>
    </template>

    <template v-if="detail.type !== 'series' || episode">
      <div class="phead">
        <h3>Pilih sumber</h3>
        <button class="reload" :disabled="pending > 0" title="Ambil ulang sumber dari semua ekstensi" @click="findStreams(...lastFind, true)">
          <Icon name="refresh" :size="18" />Muat ulang
        </button>
      </div>
      <p v-if="pending && !groups.length" class="none">Mencari sumber di semua ekstensi…</p>
      <p v-else-if="sheetError" class="none">{{ sheetError }}</p>
      <div v-if="groups.length" class="seasons" role="tablist" aria-label="Ekstensi sumber">
        <button v-for="g in groups" :key="g.id" role="tab" :aria-selected="pickTab === g.id" :class="{ on: pickTab === g.id }" @click="pickTab = g.id">
          {{ g.name }} <small>{{ g.list.length }}</small>
        </button>
        <span v-if="pending" class="more-coming"><Icon name="sync" :size="16" class="spinning" />{{ pending }} ekstensi lagi…</span>
      </div>
      <p v-if="tab && !tabList.length" class="none">{{ tab.note }}</p>
      <div v-if="tabRows.length" :key="pickTab" class="list">
        <button v-for="({ s, d }, i) in tabRows" :key="i" class="pick src" :title="cached.urls.has(s.url ?? '') ? 'Sudah ada di cache' : undefined" @click="play(s)">
          <span class="no">{{ i + 1 }}</span>
          <span class="q" :class="{ hd: d.quality === '4K' || d.quality === '1080p' }">{{ d.quality || "SD" }}</span>
          <span class="info">
            <strong>{{ d.head }}</strong>
            <span v-if="cached.urls.has(s.url ?? '') || d.size || d.tags.length" class="tags">
              <span v-if="cached.urls.has(s.url ?? '')" class="tag kept"><Icon name="downloadDone" :size="14" />Di cache</span>
              <span v-if="d.size" class="tag size"><Icon name="download" :size="14" />{{ d.size }}</span>
              <span v-for="t in d.tags" :key="t" class="tag">{{ t }}</span>
            </span>
            <small v-if="d.rest">{{ d.rest }}</small>
            <small v-if="d.file" class="file" :title="d.file">{{ d.file }}</small>
          </span>
          <span class="go"><Icon name="play" :size="26" /></span>
        </button>
      </div>
    </template>
  </div>

  <div v-else-if="q" class="sections">
    <CatalogRow v-for="s in searches" :key="`${s.e.id}:${s.c.type}:${q}`" :title="s.title" :source="s.e.id" :type="s.c.type"
      :catalog-id="s.c.id" :extra="`search=${encodeURIComponent(q)}`" @open="openMeta($event, s.e.id)" />
    <p v-if="!searches.length" class="none">Ekstensi yang terpasang tidak mendukung pencarian.</p>
  </div>

  <div v-else class="sections">
    <section v-if="watching.length || bookmarked.length" class="source">
      <CatalogRow v-if="watching.length" title="Lanjutkan menonton" :items="watching" removable @open="openSaved($event as Watched)" @remove="forget($event.id)" />
      <CatalogRow v-if="bookmarked.length" title="Disimpan" :items="bookmarked" removable @open="openSaved($event as Saved)"
        @remove="toggleBookmark($event, ($event as Saved).source)" />
    </section>
    <section v-for="{ e, info } in sections" :key="e.id" class="source">
      <h2>{{ extName(e) }}</h2>
      <p v-if="!info" class="none">Memuat katalog…</p>
      <CatalogRow v-for="c in info ? rowCatalogs(info) : []" :key="`${c.type}:${c.id}`" :title="`${c.name} · ${kindName(c.type)}`"
        :source="e.id" :type="c.type" :catalog-id="c.id" :refresh="refreshN" @open="openMeta($event, e.id)" />
    </section>
  </div>

</template>

<style scoped>
.none { color: var(--on-surface-variant); font-size: 15px; padding: 8px 0; margin: 0; }
.sections { display: flex; flex-direction: column; gap: 36px; }
.source { display: flex; flex-direction: column; gap: 24px; min-width: 0; }
.source h2 { margin: 0; font-size: 24px; font-weight: 750; letter-spacing: -0.01em; }

.page { display: flex; flex-direction: column; gap: 20px; min-width: 0; }
.hero { display: flex; gap: 20px; }
.hposter { width: 200px; aspect-ratio: 2 / 3; object-fit: cover; border-radius: 16px; flex: none; }
.htext { min-width: 0; display: flex; flex-direction: column; gap: 8px; justify-content: flex-end; }
.htext h2 { margin: 0; font-size: clamp(28px, 4vw, 44px); font-weight: 800; line-height: 1.05; letter-spacing: -0.02em; }
.sub { color: var(--primary); font-weight: 600; font-size: 14px; }
.hacts { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 8px; }
.hbtn {
  height: 44px; padding: 0 20px 0 16px; border-radius: 22px; display: inline-flex; align-items: center; gap: 8px;
  font-size: 14px; font-weight: 650; background: var(--secondary-container); color: var(--on-secondary-container);
  transition: border-radius 0.3s var(--spring);
}
.hbtn:active { border-radius: 14px; }
.hbtn.filled { background: var(--primary); color: var(--on-primary); }
.hbtn { white-space: nowrap; }
@media (max-width: 600px) { .hbtn { height: 40px; padding: 0 14px 0 10px; font-size: 13px; gap: 6px; } }
.hbtn.on { background: var(--tertiary-container); color: var(--on-tertiary-container); }
.desc { margin: 4px 0 0; font-size: 14px; line-height: 1.5; color: var(--on-surface-variant); display: -webkit-box; -webkit-line-clamp: 6; max-width: 70ch; -webkit-box-orient: vertical; overflow: hidden; }
h3 { margin: 4px 0 0; font-size: 16px; font-weight: 700; }
.seasons { display: flex; gap: 8px; overflow-x: auto; scrollbar-width: none; }
.seasons button { flex: none; padding: 8px 16px; border-radius: 999px; font-size: 14px; font-weight: 600; background: var(--surface-container-highest); color: var(--on-surface-variant); }
.seasons button.on { background: var(--secondary-container); color: var(--on-secondary-container); }
.seasons small { margin-left: 4px; font-size: 12px; opacity: 0.75; }
.list { display: flex; flex-direction: column; gap: 6px; }
.pick {
  display: flex; align-items: center; gap: 14px; text-align: left; padding: 12px 16px; border-radius: 16px;
  background: var(--surface-container-high); transition: border-radius 0.3s var(--spring), background 0.2s;
}
.pick:hover { background: var(--surface-container-highest); border-radius: 24px; }
.pick strong { flex: none; font-size: 14px; font-weight: 700; min-width: 40px; }
.pick > span { flex: 1; min-width: 0; font-size: 14px; color: var(--on-surface-variant); overflow: hidden; text-overflow: ellipsis; }
.src { padding: 10px 10px 10px 12px; gap: 12px; }
.src .no { flex: none; min-width: 2ch; text-align: right; font-size: 13px; font-weight: 600; font-variant-numeric: tabular-nums; opacity: 0.7; }
.src .q {
  flex: none; min-width: 52px; height: 28px; padding: 0 8px; border-radius: 8px; display: grid; place-items: center;
  font-size: 12px; font-weight: 800; letter-spacing: 0.02em; background: var(--surface-container-highest); color: var(--on-surface-variant);
}
.src .q.hd { background: var(--tertiary-container); color: var(--on-tertiary-container); }
.src .info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.src .info strong { min-width: 0; font-size: 14px; font-weight: 650; color: var(--on-surface); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.src .info small {
  font-size: 12px; line-height: 1.4; color: var(--on-surface-variant); overflow: hidden;
  display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow-wrap: anywhere;
}
.src .tags { display: flex; flex-wrap: wrap; gap: 4px; margin: 2px 0; }
.src .tag {
  height: 22px; padding: 0 7px; border-radius: 6px; display: inline-flex; align-items: center; gap: 3px;
  font-size: 11px; font-weight: 700; background: var(--surface-container-highest); color: var(--on-surface-variant);
}
.src .tag.kept { background: var(--tertiary-container); color: var(--on-tertiary-container); }
.src .tag.size { background: var(--secondary-container); color: var(--on-secondary-container); font-variant-numeric: tabular-nums; }
.src .file { font-family: ui-monospace, monospace; font-size: 11px; opacity: 0.8; -webkit-line-clamp: 1; }
/* tonal at rest, filled on hover: visible on any row without shouting over the list */
.src .go {
  flex: none; width: 44px; height: 44px; border-radius: 22px; display: grid; place-items: center;
  background: var(--secondary-container); color: var(--on-secondary-container);
  transition: border-radius 0.3s var(--spring), background 0.2s, color 0.2s;
}
.src:hover .go { border-radius: 14px; background: var(--primary); color: var(--on-primary); }

@media (max-width: 600px) {
  .hero { gap: 14px; }
  .hposter { width: 120px; }
  .htext h2 { font-size: 24px; }
  .desc { -webkit-line-clamp: 4; }
}
/* the title's artwork behind the whole detail page, edge to edge, fading into the surface below */
.page { position: relative; isolation: isolate; }
.backdrop {
  position: absolute; z-index: -1; top: 0; left: calc(-1 * var(--pad-l, 16px)); right: calc(-1 * var(--pad-r, 8px));
  height: min(85vh, 760px); background: center 20% / cover no-repeat;
  -webkit-mask-image: linear-gradient(#000 35%, transparent); mask-image: linear-gradient(#000 35%, transparent);
}
.backdrop::after {
  content: ""; position: absolute; inset: 0;
  background: linear-gradient(90deg, color-mix(in srgb, var(--surface) 92%, transparent) 0%, color-mix(in srgb, var(--surface) 55%, transparent) 60%, transparent),
    linear-gradient(color-mix(in srgb, var(--surface) 30%, transparent), transparent 40%);
}
/* a poster only (no wide art): blurred so the portrait crop doesn't look stretched */
.backdrop.blur { filter: blur(28px) saturate(1.2); transform: scale(1.1); }
.page .hero { min-height: min(45vh, 420px); align-items: flex-end; padding-top: 24px; }
.page .desc { color: var(--on-surface); max-width: 70ch; }
.phead { display: flex; align-items: center; gap: 12px; }
.phead h3 { flex: 1; }
.reload {
  display: inline-flex; align-items: center; gap: 6px; height: 36px; padding: 0 14px 0 10px; border-radius: 18px; font-size: 14px; font-weight: 600;
  background: var(--secondary-container); color: var(--on-secondary-container); transition: background 0.2s, opacity 0.2s;
}
.reload:hover:not(:disabled) { background: var(--surface-container-highest); }
.reload:disabled { opacity: 0.6; }
.spinning { animation: spin 0.9s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
.more-coming { display: inline-flex; align-items: center; gap: 6px; padding: 0 8px; font-size: 13px; color: var(--on-surface-variant); }
.epsel { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 12px; }
.epnav { margin-left: auto; display: flex; gap: 4px; }
.epnav button {
  width: 40px; height: 40px; border-radius: 20px; display: grid; place-items: center; color: var(--on-surface);
  background: var(--surface-container-high); transition: background 0.2s, opacity 0.2s;
}
.epnav button:hover:not(:disabled) { background: var(--surface-container-highest); }
.epnav button:disabled { opacity: 0.35; cursor: default; }
.epnav .fwd :deep(svg) { transform: scaleX(-1); }
@media (hover: none) { .epnav { display: none; } }
.sel { width: min(100%, 320px); }
/* episodes: a horizontal strip of cards like the catalog rows */
.eps {
  display: flex; gap: 14px; overflow-x: auto; scroll-snap-type: x proximity; padding: 2px 2px 8px;
  scrollbar-width: none; overscroll-behavior-x: contain;
}
.eps::-webkit-scrollbar { display: none; }
.ep { flex: none; width: 220px; display: flex; flex-direction: column; gap: 3px; text-align: left; scroll-snap-align: start; }
.ethumb {
  position: relative; aspect-ratio: 16 / 9; border-radius: 16px; overflow: hidden; display: grid; place-items: center; margin-bottom: 6px;
  background: var(--secondary-container); color: var(--on-secondary-container);
  transition: border-radius 0.4s var(--spring), transform 0.45s var(--spring), box-shadow 0.2s;
}
.ethumb img { width: 100%; height: 100%; object-fit: cover; }
.ep:hover .ethumb { border-radius: 24px; transform: scale(1.03); }
.ep.on .ethumb { box-shadow: 0 0 0 3px var(--primary); }
.enum {
  position: absolute; left: 8px; bottom: 8px; padding: 2px 8px; border-radius: 8px; font-size: 12px; font-weight: 700;
  background: rgb(0 0 0 / 0.65); color: #fff;
}
.saved {
  position: absolute; top: 8px; left: 8px; width: 24px; height: 24px; border-radius: 12px; display: grid; place-items: center;
  background: var(--tertiary-container); color: var(--on-tertiary-container);
}
.eplay { position: absolute; inset: 0; display: grid; place-items: center; background: rgb(0 0 0 / 0.35); color: #fff; }
.ep strong { font-size: 14px; font-weight: 650; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ep.on strong { color: var(--primary); }
.ep span:not(.enum):not(.eplay) { font-size: 12px; color: var(--on-surface-variant); }
@media (max-width: 600px) { .ep { width: 170px; } }
.edefault {
  position: absolute; inset: 0; display: grid; place-items: center; color: #fff;
  background: var(--secondary-container) center / cover no-repeat;
}
.edefault::before { content: ""; position: absolute; inset: 0; background: rgb(0 0 0 / 0.55); backdrop-filter: blur(6px); }
.edefault :deep(svg) { position: relative; opacity: 0.9; }
</style>
