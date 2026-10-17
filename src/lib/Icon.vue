<script setup lang="ts">
import { icons, type IconName } from "./icons";

withDefaults(defineProps<{ name: IconName; size?: number }>(), { size: 24 });
</script>

<template>
  <span class="icon" :style="{ '--size': `${size}px` }" aria-hidden="true" v-html="icons[name]"></span>
</template>

<style scoped>
.icon { display: inline-grid; width: var(--size); height: var(--size); flex: none; }
.icon :deep(svg) { width: 100%; height: 100%; fill: currentColor; }
</style>
