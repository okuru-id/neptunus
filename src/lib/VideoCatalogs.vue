<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import Icon from "./Icon.vue";
import CatalogRow from "./CatalogRow.vue";
import { extName, type ExtInfo } from "./ext";
import {
  allStreams, subtitleQuery, CINEMETA, meta, rowCatalogs, searchCatalog, sourceInfo, streamScore, videoSources,
  type Episode, type Meta, type SourceInfo, type SourceStream, type StreamGroup, type SubtitleQuery,
} from "./videoext";

// Video › Ekstensi: every addon with catalogs stacked as its own section, each catalog a
// horizontal row. Playing asks all addons for streams, not just the one the title came from.
const props = defineProps<{ exts: ExtInfo[]; query: string }>();
const emit = defineEmits<{ play: [v: { path: string; title: string; sub?: SubtitleQuery }]; changed: [ExtInfo[]] }>();

const CINEMETA_KEY = "neptunus.cinemetaAdded";
const infos = reactive<Record<string, SourceInfo | "error">>({});
const isCinemeta = (e: ExtInfo) => String(e.values?.manifestUrl ?? "").includes("cinemeta");
const sources = computed(() =>
  videoSources(props.exts).sort((a, b) => Number(isCinemeta(b)) - Number(isCinemeta(a))));
const ready = (id: string) => { const i = infos[id]; return i && i !== "error" ? i : undefined; };
const sections = computed(() =>
  sources.value.map((e) => ({ e, info: ready(e.id) })).filter((s) => !s.info || rowCatalogs(s.info).length));
const kindName = (t: string) => (t === "movie" ? "Film" : t === "series" ? "Serial" : t);

// header search, debounced: each addon's search catalogs become rows of results
const q = ref("");
let debounce: ReturnType<typeof setTimeout>;
watch(() => props.query, (v) => { clearTimeout(debounce); debounce = setTimeout(() => (q.value = v.trim()), 450); });
const searches = computed(() =>
  sources.value.flatMap((e) => {
    const info = ready(e.id);
    if (!info) return [];
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
    if (open.value?.m === m && full) detail.value = { ...m, ...full };
  } catch {}
  if (m.type === "series") season.value = seasons.value.find(([s]) => s > 0)?.[0] ?? seasons.value[0]?.[0] ?? 1;
  else findStreams(m.type, detail.value?.id ?? m.id);
}

async function findStreams(type: string, id: string) {
  const mine = ++ask;
  found.value = null;
  sheetError.value = "";
  const all = sources.value.map((e) => ({ id: e.id, name: extName(e), info: ready(e.id) }));
  const list = await allStreams(all, type, id);
  if (mine !== ask) return;
  // playable first (h264/web-ready), padded & torrent-only sink to the bottom; addons with nothing last
  for (const g of list) g.list.sort((a, b) => streamScore(a) - streamScore(b));
  found.value = list.sort((a, b) => Number(!a.list.length) - Number(!b.list.length));
  if (!list.length) sheetError.value = "Belum ada ekstensi yang menyediakan sumber video.";
}

// the picker groups streams by the addon they came from, one tab each
const pickTab = ref("");
const groups = computed(() => found.value ?? []);
const tab = computed(() => groups.value.find((g) => g.id === pickTab.value) ?? groups.value[0]);
const tabList = computed(() => tab.value?.list ?? []);
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

function pickEpisode(ep: Episode) {
  episode.value = ep;
  findStreams("series", ep.id);
}

function play(s: SourceStream) {
  const m = detail.value!;
  const ep = episode.value;
  const title = ep ? `${m.name} · S${ep.season ?? 1}E${ep.episode ?? ""}${ep.name && !/^S\d/.test(ep.name) ? ` — ${ep.name}` : ""}` : m.name;
  const all = sources.value.map((e) => ({ id: e.id, name: extName(e), info: ready(e.id) }));
  emit("play", { path: s.url!, title, sub: subtitleQuery(all, m.type, ep?.id ?? m.id, s) });
}

function close() {
  open.value = null;
  ask++;
  requestAnimationFrame(() => scroller()?.scrollTo({ top: scrollBack }));
}
</script>

<template>
  <p v-if="!sources.length" class="none">Belum ada ekstensi video. Tambah addon Stremio di Pengaturan › Ekstensi › Video.</p>

  <div v-else-if="open && detail" class="page" :aria-label="detail.name">
    <button class="back" @click="close"><Icon name="back" :size="18" />Kembali</button>
    <div class="hero">
      <img v-if="detail.poster" :src="detail.poster" alt="" class="hposter" />
      <div class="htext">
        <h2>{{ detail.name }}</h2>
        <span class="sub">{{ [detail.releaseInfo || detail.year, kindName(detail.type)].filter(Boolean).join(" · ") }}</span>
        <p v-if="detail.description" class="desc">{{ detail.description }}</p>
      </div>
    </div>
    <template v-if="detail.type === 'series' && !episode">
      <div v-if="seasons.length > 1" class="seasons">
        <button v-for="[s] in seasons" :key="s" :class="{ on: season === s }" @click="season = s">{{ s ? `Musim ${s}` : "Spesial" }}</button>
      </div>
      <div class="list">
        <button v-for="ep in seasons.find(([s]) => s === season)?.[1] ?? []" :key="ep.id" class="pick" @click="pickEpisode(ep)">
          <strong>E{{ ep.episode ?? "?" }}</strong>
          <span>{{ ep.name || detail.name }}</span>
        </button>
        <p v-if="!seasons.length" class="none">Daftar episode tidak tersedia.</p>
      </div>
    </template>

    <template v-else>
      <button v-if="episode" class="back" @click="episode = null; ask++"><Icon name="back" :size="18" />Episode {{ episode.episode }}</button>
      <h3>Pilih sumber</h3>
      <p v-if="found === null" class="none">Mencari sumber di semua ekstensi…</p>
      <p v-else-if="sheetError" class="none">{{ sheetError }}</p>
      <div v-if="groups.length" class="seasons" role="tablist" aria-label="Ekstensi sumber">
        <button v-for="g in groups" :key="g.id" role="tab" :aria-selected="pickTab === g.id" :class="{ on: pickTab === g.id }" @click="pickTab = g.id">
          {{ g.name }} <small>{{ g.list.length }}</small>
        </button>
      </div>
      <p v-if="tab && !tabList.length" class="none">{{ tab.note }}</p>
      <div v-if="tabList.length" :key="pickTab" class="list">
        <button v-for="(s, i) in tabList" :key="i" class="pick src" @click="play(s)">
          <span class="q" :class="{ hd: describe(s).quality === '4K' || describe(s).quality === '1080p' }">{{ describe(s).quality || "SD" }}</span>
          <span class="info">
            <strong>{{ describe(s).head }}</strong>
            <span v-if="describe(s).size || describe(s).tags.length" class="tags">
              <span v-if="describe(s).size" class="tag size"><Icon name="download" :size="14" />{{ describe(s).size }}</span>
              <span v-for="t in describe(s).tags" :key="t" class="tag">{{ t }}</span>
            </span>
            <small v-if="describe(s).rest">{{ describe(s).rest }}</small>
            <small v-if="describe(s).file" class="file" :title="describe(s).file">{{ describe(s).file }}</small>
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
    <section v-for="{ e, info } in sections" :key="e.id" class="source">
      <h2>{{ extName(e) }}</h2>
      <p v-if="!info" class="none">Memuat katalog…</p>
      <CatalogRow v-for="c in info ? rowCatalogs(info) : []" :key="`${c.type}:${c.id}`" :title="`${c.name} · ${kindName(c.type)}`"
        :source="e.id" :type="c.type" :catalog-id="c.id" @open="openMeta($event, e.id)" />
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
.src .tag.size { background: var(--secondary-container); color: var(--on-secondary-container); font-variant-numeric: tabular-nums; }
.src .file { font-family: ui-monospace, monospace; font-size: 11px; opacity: 0.8; -webkit-line-clamp: 1; }
/* tonal at rest, filled on hover: visible on any row without shouting over the list */
.src .go {
  flex: none; width: 44px; height: 44px; border-radius: 22px; display: grid; place-items: center;
  background: var(--secondary-container); color: var(--on-secondary-container);
  transition: border-radius 0.3s var(--spring), background 0.2s, color 0.2s;
}
.src:hover .go { border-radius: 14px; background: var(--primary); color: var(--on-primary); }

.back { align-self: flex-start; display: inline-flex; align-items: center; gap: 6px; padding: 8px 16px; border-radius: 999px; font-weight: 600; background: var(--secondary-container); color: var(--on-secondary-container); }
@media (max-width: 600px) {
  .hero { gap: 14px; }
  .hposter { width: 120px; }
  .htext h2 { font-size: 24px; }
  .desc { -webkit-line-clamp: 4; }
}
</style>
