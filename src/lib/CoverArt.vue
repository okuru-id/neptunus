<script setup lang="ts">
import { onMounted, onUnmounted, ref, watchEffect } from "vue";
import Icon from "./Icon.vue";
import { covers, loadCover, pending } from "./covers";
import { isClip } from "./shapes";

const props = withDefaults(defineProps<{ path: string; shape?: string }>(), { shape: "rounded" });

// Covers load lazily, once the art scrolls into view.
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
watchEffect(() => { if (seen.value) loadCover(props.path); });
</script>

<template>
  <div ref="el" :class="['art', `shape-${shape}`, { loading: pending[path] }]" :style="{ clipPath: isClip(shape) ? `url(#shape-${shape})` : undefined }">
    <img v-if="covers[path]" :src="covers[path]!" alt="" />
    <Icon v-else name="note" :size="28" />
  </div>
</template>

<style scoped>
.art {
  width: 48px; height: 48px; flex: none; overflow: hidden; display: grid; place-items: center;
  background: var(--surface-container-highest); color: var(--on-surface-variant);
}
.art img { width: 100%; height: 100%; object-fit: cover; }
.art.loading::after {
  content: ""; grid-area: 1 / 1; width: 100%; height: 100%;
  background: linear-gradient(100deg, transparent 20%, color-mix(in srgb, var(--on-surface-variant) 22%, transparent) 50%, transparent 80%);
  background-size: 200% 100%; animation: shimmer 1.2s linear infinite;
}
.art.loading :deep(.icon) { opacity: 0.5; animation: pulse 1.2s ease-in-out infinite; }
@keyframes shimmer { from { background-position: 200% 0; } to { background-position: -100% 0; } }
@keyframes pulse { 50% { opacity: 0.2; } }
@media (prefers-reduced-motion: reduce) { .art.loading::after, .art.loading :deep(.icon) { animation: none; } }
.shape-rounded { border-radius: 12px; }
.shape-squircle { border-radius: 30%; }
.shape-arch { border-radius: 50% 50% 14px 14px; }
.shape-circle { border-radius: 50%; }
</style>
