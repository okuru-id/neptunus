<script setup lang="ts">
import { nextTick, onUnmounted, ref } from "vue";
import Icon from "./Icon.vue";
import { extName, type ExtInfo } from "./ext";

// ⋮ menu on a song row: re-download with a chosen extension, or delete the song.
// `download`: a song not downloaded yet; the trigger is a download button and there is no delete entry.
// `failed`: why the last download failed; the button then offers to try again.
// `label`: shown next to the icon, making the trigger a pill button (a playlist's "download all").
// Default slot: more entries at the top of the menu (on phones the row's own buttons move in here); it gets
// `pick(fn)`, which closes the menu and runs fn.
defineProps<{ title: string; downloaders: ExtInfo[]; busy?: number | null; download?: boolean; failed?: string; label?: string }>();
const emit = defineEmits<{ redownload: [string, string | null]; remove: [] }>();

const open = ref(false);
// opens upward when there is more room above the button than below (inside the scrolling page, above the
// player), and never grows past that room
const up = ref(false);
const maxH = ref(0);
// phones: a bottom sheet over a scrim instead of a dropdown, so a long menu never runs off the screen
const sheetMq = matchMedia("(max-width: 600px)");
const sheet = ref(false);
// extension whose quality list is shown (second step of re-download)
const ext = ref<ExtInfo | null>(null);
const root = ref<HTMLElement>();
const menu = ref<HTMLElement>();
function outside(e: PointerEvent) {
  if (!root.value?.contains(e.target as Node)) close();
}
function toggle() {
  open.value = !open.value;
  if (!open.value) return window.removeEventListener("pointerdown", outside);
  window.addEventListener("pointerdown", outside);
  up.value = false;
  maxH.value = 0;
  ext.value = null;
  sheet.value = sheetMq.matches;
  if (sheet.value) return;
  nextTick(() => {
    const btn = root.value!.getBoundingClientRect();
    const h = menu.value?.offsetHeight ?? 0;
    const main = root.value!.closest("main");
    const area = main?.getBoundingClientRect() ?? { top: 0, bottom: window.innerHeight };
    // main's bottom padding is the space the floating player covers
    const bottom = area.bottom - (main ? parseFloat(getComputedStyle(main).paddingBottom) : 0);
    const below = bottom - btn.bottom - 12;
    const above = btn.top - area.top - 12;
    up.value = h > below && above > below;
    maxH.value = up.value ? above : below;
  });
}
function close() {
  open.value = false;
  window.removeEventListener("pointerdown", outside);
}
function pick(f: () => void) {
  close();
  f();
}
onUnmounted(close);
</script>

<template>
  <div ref="root" class="song-menu" :class="{ open }">
    <button class="icon-btn" :class="{ labeled: label }" :disabled="download && busy != null"
      :title="busy != null ? `Mengunduh${download ? '' : ' ulang'} ${Math.round(busy)}%` : failed ? `Gagal: ${failed}` : download ? 'Unduh lagu ini' : 'Lainnya'"
      :aria-label="`${download ? 'Unduh' : 'Lainnya untuk'} ${title}`" :aria-expanded="open" @click="toggle">
      <span v-if="busy != null" class="pct">{{ Math.round(busy) }}%</span>
      <Icon v-else :name="failed ? 'refresh' : download ? 'download' : 'moreVert'" :size="20" />
      <template v-if="label">{{ label }}</template>
    </button>
    <div v-if="open && sheet" class="scrim" @click="close"></div>
    <div v-if="open" ref="menu" class="menu" :class="{ up, sheet }" :style="maxH ? { maxHeight: `${maxH}px` } : undefined" role="menu">
      <p v-if="sheet" class="sheet-title">{{ title }}</p>
      <template v-if="!ext && $slots.default">
        <slot :pick="pick" />
        <hr v-if="downloaders.length || !download" />
      </template>
      <template v-if="ext">
        <p>Kualitas · {{ extName(ext) }}</p>
        <button role="menuitem" @click="pick(() => emit('redownload', ext!.id, null))"><Icon name="download" :size="20" />Otomatis</button>
        <button v-for="q in ext.qualityOptions" :key="q.id" role="menuitem" :title="q.description" @click="pick(() => emit('redownload', ext!.id, q.id))">
          <Icon name="download" :size="20" />{{ q.label }}
        </button>
      </template>
      <template v-else-if="downloaders.length">
        <p>{{ download ? 'Unduh dengan' : 'Unduh ulang dengan' }}</p>
        <button v-for="d in downloaders" :key="d.id" role="menuitem" :disabled="busy != null" @click="d.qualityOptions?.length ? (ext = d) : pick(() => emit('redownload', d.id, null))">
          <img v-if="d.iconData" :src="d.iconData" alt="" /><Icon v-else name="download" :size="20" />{{ extName(d) }}
        </button>
        <hr v-if="!download" />
      </template>
      <p v-else-if="download">Belum ada ekstensi unduhan. Pasang di Pengaturan › Ekstensi.</p>
      <button v-if="!ext && !download" role="menuitem" class="danger" @click="pick(() => emit('remove'))"><Icon name="trash" :size="20" />Hapus lagu</button>
    </div>
  </div>
</template>

<style scoped>
.song-menu { position: relative; flex: none; }
.icon-btn {
  width: 40px; height: 40px; border-radius: 20px; display: inline-grid; place-items: center;
  color: var(--on-surface-variant); transition: background 0.2s, border-radius 0.3s var(--spring);
}
.icon-btn:hover, .open .icon-btn { background: var(--surface-container-highest); }
/* same as the app's .btn.tonal.big */
.icon-btn.labeled {
  width: auto; height: 64px; padding: 0 32px 0 24px; border-radius: 32px; display: inline-flex; gap: 8px;
  font-size: 18px; font-weight: 600; font-variation-settings: "wdth" 110;
  background: var(--secondary-container); color: var(--on-secondary-container);
}
.icon-btn.labeled:active { border-radius: 12px; }
.pct { font-size: 11px; font-weight: 700; font-variant-numeric: tabular-nums; color: var(--primary); }
.menu {
  position: absolute; right: 0; top: calc(100% + 4px); z-index: 20; min-width: 220px; padding: 8px 0; border-radius: 16px;
  background: var(--surface-container-high); box-shadow: 0 8px 28px rgb(0 0 0 / 0.3);
  display: flex; flex-direction: column; max-height: 60vh; overflow-y: auto; animation: drop 0.2s var(--emph);
}
.menu > *, .menu :slotted(*) { flex: none; } /* a capped menu scrolls; its entries keep their size */
.menu.up { top: auto; bottom: calc(100% + 4px); transform-origin: bottom right; animation-name: rise; }
@keyframes drop { from { opacity: 0; transform: translateY(-6px) scale(0.97); } }
@keyframes rise { from { opacity: 0; transform: translateY(6px) scale(0.97); } }
.menu p { margin: 4px 16px 6px; font-size: 12px; font-weight: 600; color: var(--on-surface-variant); }
.menu button { flex: none; display: flex; align-items: center; gap: 12px; height: 44px; padding: 0 16px; font-size: 14px; text-align: left; color: var(--on-surface); }
.menu button:hover:not(:disabled) { background: color-mix(in srgb, var(--on-surface) 8%, transparent); }
.menu button:disabled { opacity: 0.5; }
.menu img { width: 20px; height: 20px; border-radius: 5px; object-fit: cover; }
.scrim { position: fixed; inset: 0; z-index: 49; background: rgb(0 0 0 / 0.45); animation: fade 0.2s var(--emph); }
.menu.sheet {
  position: fixed; left: max(8px, env(safe-area-inset-left)); right: max(8px, env(safe-area-inset-right)); top: auto;
  bottom: calc(8px + env(safe-area-inset-bottom)); z-index: 50; min-width: 0; max-height: 75vh; padding: 20px 0 12px; border-radius: 28px;
  transform-origin: bottom center; animation-name: sheet-up;
}
/* drag-handle look, as on an M3 bottom sheet */
.menu.sheet::before { content: ""; position: absolute; top: 8px; left: 50%; width: 32px; height: 4px; margin-left: -16px; border-radius: 2px; background: var(--outline-variant); }
.menu .sheet-title { margin: 4px 16px 8px; font-size: 15px; font-weight: 700; color: var(--on-surface); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.menu.sheet button, .menu.sheet :slotted(button) { height: 52px; flex: none; }
@keyframes sheet-up { from { opacity: 0; transform: translateY(32px); } }
@keyframes fade { from { opacity: 0; } }
/* entries from the slot are written in the parent's scope */
.menu :slotted(p) { margin: 4px 16px 6px; font-size: 12px; font-weight: 600; color: var(--on-surface-variant); }
.menu :slotted(button) { flex: none; display: flex; align-items: center; gap: 12px; height: 44px; padding: 0 16px; font-size: 14px; text-align: left; color: var(--on-surface); }
.menu :slotted(button:hover:not(:disabled)) { background: color-mix(in srgb, var(--on-surface) 8%, transparent); }
.menu :slotted(button:disabled) { opacity: 0.5; }
.menu hr { width: 100%; margin: 6px 0; border: 0; border-top: 1px solid var(--outline-variant); }
.menu .danger { color: #ffb4ab; }
@media (prefers-color-scheme: light) { .menu .danger { color: #ba1a1a; } }
</style>
