<script setup lang="ts">
import { getCurrentWindow } from "@tauri-apps/api/window";
import { onMounted, onUnmounted, ref } from "vue";
import Icon from "./Icon.vue";

// Custom window frame: the system decorations are disabled in tauri.conf.json. On macOS the native traffic
// lights sit on the left of this bar instead (tauri.macos.conf.json), so only the settings button is drawn.
const win = getCurrentWindow();
// macOS draws its own traffic lights; iOS/Android have no window to minimize or close
const native = ["mac", "mobile"].some((c) => document.documentElement.classList.contains(c));
defineEmits<{ settings: [] }>();
const maximized = ref(false);

const sync = async () => {
  maximized.value = await win.isMaximized();
  // square corners when maximized, rounded M3 frame otherwise
  document.documentElement.classList.toggle("maximized", maximized.value);
};
let off: Promise<() => void> | undefined;
onMounted(() => {
  sync();
  off = win.onResized(sync);
});
onUnmounted(() => off?.then((f) => f()));
</script>

<template>
  <header class="titlebar" :data-tauri-drag-region="native ? undefined : ''">
    <!-- pointer-events: none so clicks on the brand still land on the drag region -->
    <div class="brand">
      <span class="mark" :style="{ clipPath: 'url(#shape-cookie9)' }"><Icon name="note" :size="18" /></span>
      <span class="name">Neptunus</span>
    </div>
    <div class="controls">
      <slot />
      <button @click="$emit('settings')" title="Pengaturan" aria-label="Pengaturan">
        <Icon name="settings" :size="18" />
      </button>
      <template v-if="!native">
      <button @click="win.minimize()" title="Minimalkan" aria-label="Minimalkan">
        <Icon name="minimize" :size="18" />
      </button>
      <button @click="win.toggleMaximize()" :title="maximized ? 'Pulihkan' : 'Maksimalkan'" :aria-label="maximized ? 'Pulihkan' : 'Maksimalkan'">
        <Icon :name="maximized ? 'restore' : 'maximize'" :size="16" />
      </button>
      <button class="close" @click="win.close()" title="Tutup" aria-label="Tutup">
        <Icon name="close" :size="18" />
      </button>
      </template>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  position: fixed; inset: 0 0 auto 0; z-index: 30; height: var(--titlebar);
  display: flex; align-items: center; justify-content: space-between;
  padding: 0 8px 0 24px; user-select: none; -webkit-user-select: none;
  /* sits on the darker window frame (body background) */
}
.brand { display: flex; align-items: center; gap: 10px; pointer-events: none; }
.mark { width: 30px; height: 30px; display: grid; place-items: center; background: var(--tertiary-container); color: var(--on-tertiary-container); }
.name { font-size: 17px; font-weight: 800; letter-spacing: -0.02em; font-variation-settings: "wdth" 118; color: var(--on-surface); }
.controls { display: flex; gap: 4px; margin-left: auto; }
.controls button {
  width: 44px; height: 30px; border-radius: 15px; display: grid; place-items: center;
  color: var(--on-surface-variant);
  transition: background 0.15s, color 0.15s, border-radius 0.3s var(--spring);
}
.controls button:hover { background: var(--surface-container-highest); color: var(--on-surface); }
.controls button:active { border-radius: 8px; }
.controls .close:hover { background: #b3261e; color: #fff; }
/* a desktop window as narrow as a phone: the app name makes room for the window buttons */
@media (max-width: 440px) { :root:not(.mobile) .name { display: none; } }
</style>

<!-- not scoped: `:global(html.mac) .titlebar` in a scoped block compiles to plain `html.mac` -->
<style>
/* room for the traffic lights (x 20 + three 20px-spaced buttons) */
html.mac .titlebar { padding-left: 88px; }
/* below the status bar / notch, clear of rounded corners in landscape */
html.mobile .titlebar {
  padding: env(safe-area-inset-top) max(8px, env(safe-area-inset-right)) 0 max(16px, env(safe-area-inset-left));
}
html.mobile .titlebar .controls button { width: 44px; height: 40px; border-radius: 20px; }
</style>
