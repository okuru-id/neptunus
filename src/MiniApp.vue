<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { onMounted, onUnmounted, ref } from "vue";
import Icon from "./lib/Icon.vue";
import WavyProgress from "./lib/WavyProgress.vue";
import { applyTheme, DEFAULT_SEED, seedFromImage } from "./lib/theme";
import { CLIP_SHAPES } from "./lib/shapes";

// Tray popover. Owns no playback state: the main window broadcasts `np-state`,
// and controls are sent back through the same events the OS media keys use.
type NowPlaying = { title: string; artist: string; album: string; cover: string | null; paused: boolean; position: number; duration: number };

const np = ref<NowPlaying | null>(null);
const scrubbing = ref<number | null>(null);
let lastCover: string | null | undefined;

applyTheme(DEFAULT_SEED);
const sub = listen<NowPlaying | null>("np-state", ({ payload }) => {
  np.value = payload;
  if (payload?.cover !== lastCover) {
    lastCover = payload?.cover;
    const c = payload?.cover;
    (c ? seedFromImage(c).catch(() => DEFAULT_SEED) : Promise.resolve(DEFAULT_SEED)).then((s) => applyTheme(s));
  }
});
onMounted(() => emit("np-request"));
onUnmounted(() => sub.then((off) => off()));

const key = (k: "toggle" | "prev" | "next") => emit("media-key", k);
const seek = (v: number) => {
  emit("media-seek", v);
  scrubbing.value = null;
};
const close = () => getCurrentWindow().hide();
const fmt = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
</script>

<template>
  <svg class="defs" aria-hidden="true">
    <defs><clipPath id="shape-cookie9" clipPathUnits="objectBoundingBox"><path :d="CLIP_SHAPES.cookie9" /></clipPath></defs>
  </svg>

  <div class="mini">
    <template v-if="np">
      <div class="top">
        <div class="art">
          <img v-if="np.cover" :src="np.cover" alt="" />
          <Icon v-else name="note" :size="32" />
        </div>
        <div class="meta">
          <strong>{{ np.title }}</strong>
          <span>{{ np.artist }}</span>
        </div>
        <button class="icon-btn" @click="invoke('open_main')" title="Buka PixelPlay"><Icon name="openMain" :size="20" /></button>
        <button class="icon-btn" @click="close" title="Tutup" aria-label="Tutup"><Icon name="close" :size="20" /></button>
      </div>
      <div class="seek">
        <span>{{ fmt(scrubbing ?? np.position) }}</span>
        <WavyProgress
          :value="scrubbing ?? np.position"
          :max="np.duration"
          :playing="!np.paused"
          :height="22"
          @scrub="scrubbing = $event"
          @seek="seek"
        />
        <span>{{ fmt(np.duration) }}</span>
      </div>
      <div class="transport">
        <button class="icon-btn tonal" @click="key('prev')" title="Sebelumnya"><Icon name="prev" :size="24" /></button>
        <button class="play" :class="{ playing: !np.paused }" @click="key('toggle')" :title="np.paused ? 'Putar' : 'Jeda'">
          <Icon :name="np.paused ? 'play' : 'pause'" :size="28" />
        </button>
        <button class="icon-btn tonal" @click="key('next')" title="Berikutnya"><Icon name="next" :size="24" /></button>
      </div>
    </template>
    <div v-else class="idle">
      <button class="icon-btn corner" @click="close" title="Tutup" aria-label="Tutup"><Icon name="close" :size="20" /></button>
      <div class="mark" :style="{ clipPath: 'url(#shape-cookie9)' }"><Icon name="note" :size="28" /></div>
      <p>Tidak ada lagu yang diputar</p>
      <button class="btn" @click="invoke('open_main')"><Icon name="openMain" :size="18" />Buka PixelPlay</button>
    </div>
  </div>
</template>

<style>
html, body { margin: 0; height: 100%; background: transparent; overflow: hidden; }
body {
  font-family: "Roboto Flex Variable", system-ui, sans-serif; color: var(--on-surface);
  -webkit-font-smoothing: antialiased; user-select: none; -webkit-user-select: none;
}
button { font: inherit; color: inherit; border: 0; background: none; cursor: pointer; padding: 0; }
:focus-visible { outline: 3px solid var(--primary); outline-offset: 2px; }
</style>

<style scoped>
.defs { position: absolute; width: 0; height: 0; }

.mini {
  --spring: cubic-bezier(0.3, 1.3, 0.5, 1);
  box-sizing: border-box; height: 100vh; padding: 16px; border-radius: 28px;
  background: var(--surface-container-high); box-shadow: inset 0 0 0 1px var(--outline-variant);
  display: flex; flex-direction: column; justify-content: space-between;
}
.top { display: flex; align-items: center; gap: 12px; }
.top .icon-btn + .icon-btn { margin-left: -8px; }
.art {
  width: 56px; height: 56px; border-radius: 16px; overflow: hidden; flex: none; display: grid; place-items: center;
  background: var(--primary-container); color: var(--on-primary-container);
}
.art img { width: 100%; height: 100%; object-fit: cover; }
.meta { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
.meta strong { font-size: 16px; font-weight: 700; font-variation-settings: "wdth" 110; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.meta span { font-size: 13px; color: var(--on-surface-variant); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.seek { display: flex; align-items: center; gap: 10px; font-size: 11px; font-weight: 600; color: var(--on-surface-variant); font-variant-numeric: tabular-nums; }
.transport { display: flex; align-items: center; justify-content: center; gap: 10px; }
.icon-btn {
  width: 40px; height: 40px; border-radius: 20px; display: grid; place-items: center; flex: none;
  color: var(--on-surface-variant); transition: background 0.2s, border-radius 0.3s var(--spring);
}
.icon-btn:hover { background: var(--surface-container-highest); }
.icon-btn:active { border-radius: 10px; }
.icon-btn.tonal { width: 44px; height: 44px; background: var(--secondary-container); color: var(--on-secondary-container); }
.play {
  width: 68px; height: 48px; border-radius: 24px; display: grid; place-items: center;
  background: var(--primary); color: var(--on-primary);
  transition: border-radius 0.45s var(--spring), transform 0.3s var(--spring);
}
.play.playing { border-radius: 16px; }
.play:active { transform: scale(0.94); }
.idle { position: relative; height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; }
.corner { position: absolute; top: -4px; right: -4px; }
.idle p { margin: 0; font-size: 14px; color: var(--on-surface-variant); }
.mark { width: 48px; height: 48px; display: grid; place-items: center; background: var(--tertiary-container); color: var(--on-tertiary-container); }
.btn {
  display: inline-flex; align-items: center; gap: 6px; height: 36px; padding: 0 16px 0 12px; border-radius: 18px;
  background: var(--primary); color: var(--on-primary); font-size: 13px; font-weight: 600;
  transition: border-radius 0.3s var(--spring);
}
.btn:active { border-radius: 10px; }
</style>
