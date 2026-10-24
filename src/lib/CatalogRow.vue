<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import Icon from "./Icon.vue";
import { catalog, type Meta } from "./videoext";
import { cached } from "./vcache";

// One catalog as a horizontal strip of posters. Loads right away: the answer usually comes from the
// app database, so waiting for the row to scroll into view only made rows blink in while scrolling.
// `items` instead of a catalog: a row of the viewer's own titles (Lanjutkan menonton, Disimpan), each with
// an optional line under its name and how far it was watched (0-1); `removable` adds a remove button.
export type RowItem = Meta & { note?: string; progress?: number };
const props = defineProps<{ title: string; source?: string; type?: string; catalogId?: string; extra?: string; refresh?: number; items?: RowItem[]; removable?: boolean }>();
const emit = defineEmits<{ open: [m: Meta]; remove: [m: Meta] }>();

const strip = ref<HTMLElement>();
const metas = ref<Meta[] | null>(null);
const error = ref("");

async function load(force = false) {
  if (props.items) return;
  if (!force) metas.value = null; // a reload keeps the old posters up until the new list is in
  error.value = "";
  try {
    metas.value = await catalog(props.source!, props.type!, props.catalogId!, props.extra ?? "", force);
  } catch (e) {
    error.value = String(e);
    metas.value = [];
  }
}

onMounted(() => load());
const list = computed<RowItem[] | null>(() => props.items ?? metas.value);

// A catalog brings 50-100 titles and a page stacks several, but a phone shows three posters a row: render
// a handful and add more as the strip nears its end, or opening the catalogs builds hundreds of cards at once.
const PAGE = 15;
const shown = ref(PAGE);
const visible = computed(() => list.value?.slice(0, shown.value) ?? null);
watch(list, () => (shown.value = PAGE));
function more() {
  const s = strip.value;
  if (s && list.value && shown.value < list.value.length && s.scrollLeft + s.clientWidth * 2 >= s.scrollWidth) shown.value += PAGE;
}
watch(() => props.extra, () => metas.value !== null && load());
// "Muat ulang": ask the addon again and refresh the database copy
watch(() => props.refresh, () => load(true));

const scroll = (dir: number) => strip.value?.scrollBy({ left: dir * strip.value.clientWidth * 0.8, behavior: "smooth" });
</script>

<template>
  <section class="row" v-show="list === null || list.length || error">
    <header>
      <h3>{{ title }}</h3>
      <div class="nav">
        <button title="Geser kiri" @click="scroll(-1)"><Icon name="back" :size="20" /></button>
        <button title="Geser kanan" class="fwd" @click="scroll(1)"><Icon name="back" :size="20" /></button>
      </div>
    </header>
    <p v-if="error" class="note">Tidak bisa dimuat ({{ error }}). <button class="retry" @click="load(true)">Coba lagi</button></p>
    <div v-else ref="strip" class="strip" @scroll.passive="more">
      <template v-if="visible">
        <div v-for="m in visible" :key="m.id" class="card" :class="{ cached: cached.ids.has(m.id) }">
          <button class="open" :title="m.name" @click="emit('open', m)">
            <div class="poster" :class="{ wide: m.posterShape === 'landscape' }">
              <img v-if="m.poster" :src="m.poster" loading="lazy" decoding="async" alt="" />
              <Icon v-else name="movie" :size="32" />
              <span v-if="cached.ids.has(m.id)" class="saved" title="Sudah ada di cache"><Icon name="download" :size="14" /></span>
              <span v-if="m.progress" class="progress"><i :style="{ width: `${Math.round(m.progress * 100)}%` }"></i></span>
            </div>
            <strong>{{ m.name }}</strong>
            <span>{{ m.note ?? (m.releaseInfo || m.year || (m.type === "series" ? "Serial" : "")) }}</span>
          </button>
          <button v-if="removable" class="rm" :title="`Hapus ${m.name} dari daftar`" :aria-label="`Hapus ${m.name} dari daftar`" @click="emit('remove', m)">
            <Icon name="close" :size="16" />
          </button>
        </div>
      </template>
      <div v-else v-for="i in 6" :key="i" class="card ghost"><div class="poster"></div></div>
    </div>
  </section>
</template>

<style scoped>
/* rows out of view skip layout and paint until scrolled near */
.row { display: flex; flex-direction: column; gap: 10px; min-width: 0; content-visibility: auto; contain-intrinsic-size: auto 300px; }
header { display: flex; align-items: center; gap: 8px; }
h3 { flex: 1; margin: 0; font-size: 18px; font-weight: 700; }
.nav { display: flex; gap: 4px; }
.nav button {
  width: 36px; height: 36px; border-radius: 18px; display: grid; place-items: center; color: var(--on-surface-variant);
  transition: background 0.2s;
}
.nav button:hover { background: var(--surface-container-highest); }
.nav .fwd :deep(svg) { transform: scaleX(-1); }
@media (hover: none) { .nav { display: none; } }
.strip {
  display: flex; gap: 14px; overflow-x: auto; scroll-snap-type: x proximity; padding-bottom: 6px;
  scrollbar-width: none; overscroll-behavior-x: contain;
}
.strip::-webkit-scrollbar { display: none; }
.card { flex: none; width: 150px; position: relative; scroll-snap-align: start; }
.open { width: 100%; display: flex; flex-direction: column; gap: 3px; text-align: left; }
.progress { position: absolute; left: 10px; right: 10px; bottom: 10px; height: 4px; border-radius: 2px; overflow: hidden; background: rgb(255 255 255 / 0.35); }
.progress i { display: block; height: 100%; border-radius: 2px; background: var(--primary); }
.rm {
  position: absolute; top: 6px; right: 6px; width: 28px; height: 28px; border-radius: 14px; display: grid; place-items: center;
  background: rgb(0 0 0 / 0.55); color: #fff; opacity: 0; transition: opacity 0.2s;
}
.card:hover .rm, .rm:focus-visible { opacity: 1; }
@media (hover: none) { .rm { opacity: 1; top: 4px; right: 4px; width: 24px; height: 24px; background: rgb(0 0 0 / 0.45); } }
.poster {
  position: relative; aspect-ratio: 2 / 3; border-radius: 16px; overflow: hidden; display: grid; place-items: center; margin-bottom: 6px;
  background: var(--secondary-container); color: var(--on-secondary-container);
  transition: border-radius 0.4s var(--spring), transform 0.45s var(--spring);
}
.poster.wide { aspect-ratio: 16 / 9; }
/* a stream of it is in the disk cache (vcache.ts) */
.card.cached .poster { outline: 3px solid var(--tertiary); outline-offset: -3px; }
.saved {
  position: absolute; top: 8px; left: 8px; width: 24px; height: 24px; border-radius: 12px; display: grid; place-items: center;
  background: var(--tertiary-container); color: var(--on-tertiary-container);
}
.card:has(.wide) { width: 240px; }
.open:hover .poster { border-radius: 28px; transform: scale(1.03); }
.poster img { width: 100%; height: 100%; object-fit: cover; }
.card strong { font-size: 14px; font-weight: 650; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.card span { font-size: 12px; color: var(--on-surface-variant); }
.card.ghost { display: flex; flex-direction: column; }
.ghost .poster { animation: pulse 1.4s ease-in-out infinite; }
@keyframes pulse { 50% { opacity: 0.5; } }
.note { margin: 0; font-size: 14px; color: var(--on-surface-variant); }
.retry { margin-left: 6px; padding: 4px 12px; border-radius: 999px; font-weight: 600; background: var(--secondary-container); color: var(--on-secondary-container); }
@media (max-width: 600px) { .card { width: 120px; } .card:has(.wide) { width: 200px; } }
</style>
