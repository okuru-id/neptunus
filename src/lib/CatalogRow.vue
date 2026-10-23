<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import Icon from "./Icon.vue";
import { catalog, type Meta } from "./videoext";

// One catalog as a horizontal strip of posters. Loads right away: the answer usually comes from the
// app database, so waiting for the row to scroll into view only made rows blink in while scrolling.
const props = defineProps<{ title: string; source: string; type: string; catalogId: string; extra?: string; refresh?: number }>();
const emit = defineEmits<{ open: [m: Meta] }>();

const strip = ref<HTMLElement>();
const metas = ref<Meta[] | null>(null);
const error = ref("");

async function load(force = false) {
  if (!force) metas.value = null; // a reload keeps the old posters up until the new list is in
  error.value = "";
  try {
    metas.value = await catalog(props.source, props.type, props.catalogId, props.extra ?? "", force);
  } catch (e) {
    error.value = String(e);
    metas.value = [];
  }
}

onMounted(() => load());
watch(() => props.extra, () => metas.value !== null && load());
// "Muat ulang": ask the addon again and refresh the database copy
watch(() => props.refresh, () => load(true));

const scroll = (dir: number) => strip.value?.scrollBy({ left: dir * strip.value.clientWidth * 0.8, behavior: "smooth" });
</script>

<template>
  <section class="row" v-show="metas === null || metas.length || error">
    <header>
      <h3>{{ title }}</h3>
      <div class="nav">
        <button title="Geser kiri" @click="scroll(-1)"><Icon name="back" :size="20" /></button>
        <button title="Geser kanan" class="fwd" @click="scroll(1)"><Icon name="back" :size="20" /></button>
      </div>
    </header>
    <p v-if="error" class="note">Tidak bisa dimuat ({{ error }}). <button class="retry" @click="load(true)">Coba lagi</button></p>
    <div v-else ref="strip" class="strip">
      <template v-if="metas">
        <button v-for="m in metas" :key="m.id" class="card" @click="emit('open', m)">
          <div class="poster" :class="{ wide: m.posterShape === 'landscape' }">
            <img v-if="m.poster" :src="m.poster" loading="lazy" alt="" />
            <Icon v-else name="movie" :size="32" />
          </div>
          <strong>{{ m.name }}</strong>
          <span>{{ m.releaseInfo || m.year || (m.type === "series" ? "Serial" : "") }}</span>
        </button>
      </template>
      <div v-else v-for="i in 6" :key="i" class="card ghost"><div class="poster"></div></div>
    </div>
  </section>
</template>

<style scoped>
.row { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
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
.card { flex: none; width: 150px; display: flex; flex-direction: column; gap: 3px; text-align: left; scroll-snap-align: start; }
.poster {
  aspect-ratio: 2 / 3; border-radius: 16px; overflow: hidden; display: grid; place-items: center; margin-bottom: 6px;
  background: var(--secondary-container); color: var(--on-secondary-container);
  transition: border-radius 0.4s var(--spring), transform 0.45s var(--spring);
}
.poster.wide { aspect-ratio: 16 / 9; }
.card:has(.wide) { width: 240px; }
.card:hover .poster { border-radius: 28px; transform: scale(1.03); }
.poster img { width: 100%; height: 100%; object-fit: cover; }
.card strong { font-size: 14px; font-weight: 650; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.card span { font-size: 12px; color: var(--on-surface-variant); }
.ghost .poster { animation: pulse 1.4s ease-in-out infinite; }
@keyframes pulse { 50% { opacity: 0.5; } }
.note { margin: 0; font-size: 14px; color: var(--on-surface-variant); }
.retry { margin-left: 6px; padding: 4px 12px; border-radius: 999px; font-weight: 600; background: var(--secondary-container); color: var(--on-secondary-container); }
@media (max-width: 600px) { .card { width: 120px; } .card:has(.wide) { width: 200px; } }
</style>
