<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watchEffect } from "vue";
import { loadQuality, longQuality, qualities, shortQuality } from "./quality";

// Small "FLAC 24/96" label on a song row; the file is read once the row scrolls into view.
const props = defineProps<{ path: string }>();
const el = ref<HTMLElement>();
const seen = ref(false);
let io: IntersectionObserver | undefined;
onMounted(() => {
  io = new IntersectionObserver(([e]) => {
    if (!e.isIntersecting) return;
    seen.value = true;
    io?.disconnect();
  });
  io.observe(el.value!);
});
onUnmounted(() => io?.disconnect());
watchEffect(() => { if (seen.value) loadQuality(props.path); });
const q = computed(() => qualities[props.path]);
</script>

<template>
  <span ref="el" class="q" :class="{ hi: q?.lossless && (q.bits ?? 0) > 16 }" :title="q ? longQuality(q) : undefined">{{ q ? shortQuality(q) : "" }}</span>
</template>

<style scoped>
.q:empty { display: none; }
.q {
  flex: none; padding: 2px 8px; border-radius: 8px; font-size: 11px; font-weight: 700; letter-spacing: 0.02em; white-space: nowrap;
  font-variant-numeric: tabular-nums; background: var(--surface-container-highest); color: var(--on-surface-variant);
}
.q.hi { background: var(--tertiary-container); color: var(--on-tertiary-container); }
</style>
