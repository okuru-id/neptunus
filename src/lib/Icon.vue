<script setup lang="ts">
import { computed } from "vue";
import { icons, type IconName } from "./icons";
import { dotIcons } from "./dotIcons";
import { uiStyle } from "./theme";

const props = withDefaults(defineProps<{ name: IconName; size?: number }>(), { size: 24 });
// the Nothing style draws the icons it has in dot matrix
const svg = computed(() => (uiStyle.value === "nothing" && dotIcons[props.name]) || icons[props.name]);
</script>

<template>
  <span class="icon" :style="{ '--size': `${size}px` }" aria-hidden="true" v-html="svg"></span>
</template>

<style scoped>
.icon { display: inline-grid; width: var(--size); height: var(--size); flex: none; }
.icon :deep(svg) { width: 100%; height: 100%; fill: currentColor; }
</style>
