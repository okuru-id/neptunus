<script setup lang="ts" generic="T extends string | number">
import { computed, nextTick, onUnmounted, ref, watch } from "vue";
import Icon from "./Icon.vue";

// In-page dropdown instead of a native <select>: WebKitGTK shows that as a GTK popup that ignores the
// app theme and whose pointer grab can get stuck under XWayland.
const props = defineProps<{ modelValue: T | null; options: { value: T; label: string }[]; placeholder?: string; label?: string }>();
const emit = defineEmits<{ "update:modelValue": [v: T] }>();

const open = ref(false);
const root = ref<HTMLElement>();
const list = ref<HTMLElement>();
const current = computed(() => props.options.find((o) => o.value === props.modelValue));

function pick(v: T) {
  open.value = false;
  if (v !== props.modelValue) emit("update:modelValue", v);
}
const outside = (e: Event) => { if (!root.value?.contains(e.target as Node)) open.value = false; };
const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); open.value = false; } };
watch(open, async (o) => {
  if (o) document.addEventListener("pointerdown", outside, true);
  else document.removeEventListener("pointerdown", outside, true);
  if (!o) return;
  await nextTick(); // the chosen entry starts in view in a long list
  const on = list.value?.querySelector<HTMLElement>(".on");
  if (on && list.value) list.value.scrollTop = on.offsetTop - list.value.clientHeight / 2 + on.clientHeight / 2; // not scrollIntoView: that moves the page too
});
onUnmounted(() => document.removeEventListener("pointerdown", outside, true));
</script>

<template>
  <div ref="root" class="dd" :class="{ open }" @keydown="onKey">
    <span v-if="label" class="lbl">{{ label }}</span>
    <button type="button" class="field" aria-haspopup="listbox" :aria-expanded="open" @click="open = !open">
      <span class="val" :class="{ ph: !current }">{{ current?.label ?? placeholder ?? "" }}</span>
      <Icon name="chevron" :size="20" class="chev" />
    </button>
    <div v-if="open" ref="list" class="menu" role="listbox">
      <button v-for="o in options" :key="o.value" type="button" role="option" :aria-selected="o.value === modelValue"
        :class="{ on: o.value === modelValue }" @click="pick(o.value)">
        <span>{{ o.label }}</span>
        <Icon v-if="o.value === modelValue" name="done" :size="18" />
      </button>
    </div>
  </div>
</template>

<style scoped>
.dd { position: relative; display: flex; flex-direction: column; gap: 6px; min-width: 0; }
.lbl { font-size: 13px; font-weight: 600; color: var(--on-surface-variant); }
.field {
  height: 48px; display: flex; align-items: center; gap: 8px; padding: 0 12px 0 16px; border-radius: 16px; text-align: left;
  font-size: 15px; font-weight: 600; color: var(--on-surface); background: var(--surface-container-high);
  transition: background 0.2s, border-radius 0.3s var(--spring);
}
.field:hover { background: var(--surface-container-highest); }
.open .field { border-radius: 16px 16px 8px 8px; background: var(--surface-container-highest); }
.val { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.val.ph { color: var(--on-surface-variant); font-weight: 500; }
.chev { flex: none; color: var(--on-surface-variant); transform: rotate(90deg); transition: transform 0.25s var(--spring); }
.open .chev { transform: rotate(-90deg); }
.menu {
  position: absolute; left: 0; right: 0; top: calc(100% + 4px); z-index: 10; max-height: min(320px, 50vh); overflow-y: auto;
  display: flex; flex-direction: column; padding: 6px; border-radius: 8px 8px 16px 16px;
  background: var(--surface-container-high); box-shadow: 0 6px 20px rgb(0 0 0 / 0.3); scrollbar-width: thin;
  animation: drop 0.18s ease-out;
}
@keyframes drop { from { opacity: 0; transform: translateY(-6px); } }
.menu button {
  display: flex; align-items: center; gap: 8px; min-height: 40px; padding: 8px 12px; border-radius: 10px; text-align: left;
  font-size: 14px; color: var(--on-surface);
}
.menu button span { flex: 1; min-width: 0; }
.menu button:hover, .menu button:focus-visible { background: var(--surface-container-highest); }
.menu button.on { background: var(--secondary-container); color: var(--on-secondary-container); font-weight: 600; }
</style>
