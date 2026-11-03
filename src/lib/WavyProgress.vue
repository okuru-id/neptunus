<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { uiStyle } from "./theme";

const props = withDefaults(defineProps<{ value: number; max: number; playing: boolean; height?: number; buffered?: number | null }>(), { height: 28, buffered: null });
const emit = defineEmits<{ scrub: [v: number]; seek: [v: number] }>();

const WAVELENGTH = 30;
const GAP = 6;

const el = ref<HTMLElement>();
const width = ref(0);
const phase = ref(0);
const amp = ref(0);
const beat = ref(0); // the Nothing bars' step, about 9 a second

const nothing = computed(() => uiStyle.value === "nothing");
const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

// The wave rolls while playing and settles flat on pause. The frame loop stops once it is flat, so a
// paused player (a film playing, say) costs nothing; playing starts it again.
let raf = 0;
let last = 0;
let ro: ResizeObserver | undefined;
function tick(t: number) {
  const dt = Math.min((t - last) / 1000, 0.1);
  last = t;
  if (!reduced) phase.value = (phase.value + dt * Math.PI * 2 * 0.8) % (Math.PI * 2);
  if (!reduced && nothing.value) beat.value = Math.floor(t / 110);
  amp.value += ((props.playing ? 1 : 0) - amp.value) * Math.min(1, dt * 5);
  if (!props.playing && amp.value < 0.005) {
    amp.value = 0;
    raf = 0;
    return;
  }
  raf = requestAnimationFrame(tick);
}
function run() {
  if (raf) return;
  last = performance.now();
  raf = requestAnimationFrame(tick);
}
watch(() => props.playing, (on) => on && run());
onMounted(() => {
  ro = new ResizeObserver(() => (width.value = el.value!.clientWidth));
  ro.observe(el.value!);
  if (props.playing) run();
});
onUnmounted(() => {
  cancelAnimationFrame(raf);
  ro?.disconnect();
});

const mid = computed(() => props.height / 2);
const played = computed(() => (props.max > 0 ? Math.min(1, props.value / props.max) * width.value : 0));
// end of the fetched-ahead part of a stream (0-1 of the track), drawn over the rest of the track
const bufEnd = computed(() => (props.buffered == null ? 0 : Math.min(width.value - 2, props.buffered * width.value)));
const wave = computed(() => {
  const a = (props.height / 7) * amp.value;
  let d = `M2 ${mid.value}`;
  for (let x = 2; x <= played.value; x += 2)
    d += `L${x} ${(mid.value + a * Math.sin((x / WAVELENGTH) * Math.PI * 2 - phase.value)).toFixed(2)}`;
  return d;
});
// the Nothing style: music bars made of dots, one column every DOT px. The played columns bounce up and
// down in steps while it plays and fall back to one dot on pause; the rest stay a dim single dot (brighter as
// far as a stream has buffered), and the playhead column is red. A column is one dashed line: a dash of 0
// with round caps draws a dot every DOT px.
const DOT = 4.5;
const ROWS = 5;
// a fixed random 0-1 for column i at step s, so every column jumps on its own like an equaliser
const rand = (i: number, s: number) => { const v = Math.sin(i * 12.9898 + s * 78.233) * 43758.5453; return v - Math.floor(v); };
const bottom = computed(() => mid.value + ((ROWS - 1) / 2) * DOT);
const cols = computed(() => {
  const s = beat.value;
  const out: { x: number; st: string; top: number }[] = [];
  for (let x = DOT / 2, i = 0; x < width.value; x += DOT, i++) {
    const on = x <= played.value;
    const level = on ? Math.round((ROWS - 1) * amp.value * rand(i, s)) : 0;
    out.push({ x, st: on ? "on" : x <= bufEnd.value ? "buf" : "off", top: bottom.value - level * DOT - 0.01 });
  }
  const head = out.findLastIndex((c) => c.st === "on");
  if (head >= 0) out[head].st = "head";
  return out;
});
const num = (e: Event) => +(e.target as HTMLInputElement).value;
</script>

<template>
  <div ref="el" class="wavy" :style="{ height: `${height}px` }">
    <svg v-if="nothing" :width="width" :height="height" aria-hidden="true">
      <line v-for="(c, i) in cols" :key="i" :class="['col', c.st]" :x1="c.x" :y1="bottom" :x2="c.x" :y2="c.top" />
    </svg>
    <svg v-else :width="width" :height="height" aria-hidden="true">
      <path v-if="played > 2" class="active" :d="wave" />
      <line v-if="played + GAP < width - 4" class="track" :x1="played + GAP" :y1="mid" :x2="width - 2" :y2="mid" />
      <line v-if="bufEnd > played + GAP" class="buffer" :x1="played + GAP" :y1="mid" :x2="bufEnd" :y2="mid" />
      <circle class="stop" :cx="width - 2" :cy="mid" r="2" />
      <rect class="thumb" :x="Math.max(0, played - 2)" :y="mid - height / 2.6" width="4" :height="height / 1.3" rx="2" />
    </svg>
    <input
      type="range"
      min="0"
      :max="max"
      step="1"
      :value="value"
      aria-label="Posisi lagu"
      @input="emit('scrub', num($event))"
      @change="emit('seek', num($event))"
    />
  </div>
</template>

<style scoped>
.wavy { position: relative; flex: 1 1 0; min-width: 0; }
svg { display: block; overflow: visible; }
.active { fill: none; stroke: var(--primary); stroke-width: 4; stroke-linecap: round; stroke-linejoin: round; }
.track { stroke: var(--secondary-container); stroke-width: 4; stroke-linecap: round; }
.buffer { stroke: var(--primary); stroke-opacity: 0.35; stroke-width: 4; stroke-linecap: round; }
.stop { fill: var(--primary); }
.thumb { fill: var(--primary); opacity: 0; transition: opacity 0.2s; }
.col { stroke-width: 2.8; stroke-linecap: round; stroke-dasharray: 0 4.5; }
.col.on { stroke: var(--on-surface); }
.col.head { stroke: var(--red, var(--primary)); stroke-width: 3.4; }
.col.buf { stroke: var(--on-surface-variant); opacity: 0.6; }
.col.off { stroke: var(--outline); opacity: 0.45; }
.wavy:hover .thumb, .wavy:has(input:focus-visible) .thumb { opacity: 1; }
input { position: absolute; inset: 0; width: 100%; height: 100%; margin: 0; opacity: 0; cursor: pointer; }
</style>
