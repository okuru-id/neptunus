<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";

const props = withDefaults(defineProps<{ value: number; max: number; playing: boolean; height?: number }>(), { height: 28 });
const emit = defineEmits<{ scrub: [v: number]; seek: [v: number] }>();

const WAVELENGTH = 30;
const GAP = 6;

const el = ref<HTMLElement>();
const width = ref(0);
const phase = ref(0);
const amp = ref(0);

const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

// The wave rolls while playing and settles flat on pause.
let raf = 0;
let ro: ResizeObserver | undefined;
onMounted(() => {
  ro = new ResizeObserver(() => (width.value = el.value!.clientWidth));
  ro.observe(el.value!);
  let last = performance.now();
  const tick = (t: number) => {
    const dt = Math.min((t - last) / 1000, 0.1);
    last = t;
    if (!reduced) phase.value = (phase.value + dt * Math.PI * 2 * 0.8) % (Math.PI * 2);
    amp.value += ((props.playing ? 1 : 0) - amp.value) * Math.min(1, dt * 5);
    raf = requestAnimationFrame(tick);
  };
  raf = requestAnimationFrame(tick);
});
onUnmounted(() => {
  cancelAnimationFrame(raf);
  ro?.disconnect();
});

const mid = computed(() => props.height / 2);
const played = computed(() => (props.max > 0 ? Math.min(1, props.value / props.max) * width.value : 0));
const wave = computed(() => {
  const a = (props.height / 7) * amp.value;
  let d = `M2 ${mid.value}`;
  for (let x = 2; x <= played.value; x += 2)
    d += `L${x} ${(mid.value + a * Math.sin((x / WAVELENGTH) * Math.PI * 2 - phase.value)).toFixed(2)}`;
  return d;
});
const num = (e: Event) => +(e.target as HTMLInputElement).value;
</script>

<template>
  <div ref="el" class="wavy" :style="{ height: `${height}px` }">
    <svg :width="width" :height="height" aria-hidden="true">
      <path v-if="played > 2" class="active" :d="wave" />
      <line v-if="played + GAP < width - 4" class="track" :x1="played + GAP" :y1="mid" :x2="width - 2" :y2="mid" />
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
.stop { fill: var(--primary); }
.thumb { fill: var(--primary); opacity: 0; transition: opacity 0.2s; }
.wavy:hover .thumb, .wavy:has(input:focus-visible) .thumb { opacity: 1; }
input { position: absolute; inset: 0; width: 100%; height: 100%; margin: 0; opacity: 0; cursor: pointer; }
</style>
