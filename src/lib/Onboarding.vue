<script lang="ts">
import { getVersion } from "@tauri-apps/api/app";
import { ref } from "vue";

const KEY = "neptunus.onboarded";
const VER_KEY = "neptunus.lastVersion";
const open = ref(false);
const step = ref(0);
type News = { version: string; summary: string; items: string[] };
const news = ref<News | null>(null);

// release-notes/vX.Y.Z.md: "# title", a one-line summary, then "- " items until "## Install"
const NOTES = import.meta.glob("../../release-notes/*.md", { query: "?raw", import: "default", eager: true }) as Record<string, string>;
function parseNotes(version: string): News {
  const md = (NOTES[`../../release-notes/v${version}.md`] ?? "").split(/^## Install/m)[0];
  const lines = md.split("\n").map((l) => l.trim());
  return {
    version,
    summary: lines.find((l) => l && !l.startsWith("#") && !l.startsWith("- ")) ?? "",
    items: lines.filter((l) => l.startsWith("- ")).map((l) => l.slice(2)),
  };
}

/** Opens the tour from the first page (Settings › Tentang replays it). */
export function showOnboarding() {
  news.value = null;
  step.value = 0;
  open.value = true;
}
/** At startup: the tour for anyone who hasn't seen it yet, otherwise what's new once after an update. */
export async function firstRun() {
  const version = await getVersion().catch(() => "");
  let seen = true, last: string | null = null;
  try { seen = !!localStorage.getItem(KEY); last = localStorage.getItem(VER_KEY); localStorage.setItem(VER_KEY, version); } catch {}
  if (!seen) showOnboarding();
  else if (version && last !== version) (news.value = parseNotes(version)), (open.value = true);
}
</script>

<script setup lang="ts">
import { openUrl } from "@tauri-apps/plugin-opener";
import { nextTick, watch } from "vue";
import Icon from "./Icon.vue";
import type { IconName } from "./icons";

// First-run tour: a few cards on what the app does, ending on the two ways to start listening.
const emit = defineEmits<{ done: [action: "folder" | "catalog"] }>();
const STEPS: { icon: IconName; shape: string; title: string; text: string }[] = [
  { icon: "note", shape: "cookie12", title: "Selamat datang di Neptunus", text: "Satu pemutar kecil untuk musik dan videomu. Warnanya mengikuti sampul lagu yang sedang diputar." },
  { icon: "folder", shape: "cookie9", title: "Library milikmu sendiri", text: "Pindai folder lagu di perangkat ini, atau putar langsung dari bucket S3. Tanpa akun, tanpa pelacakan." },
  { icon: "extension", shape: "clover4", title: "Katalog dan ekstensi", text: "Pasang ekstensi untuk menjelajahi katalog online, mengunduh lagu, dan mengambil lirik yang tersinkron." },
  { icon: "movie", shape: "cookie12", title: "Video juga ada di sini", text: "Tonton video dari folder atau S3, dan tambahkan addon Stremio untuk katalog film dan serial." },
  { icon: "play", shape: "sunny8", title: "Siap memutar", text: "Mulai dari folder musikmu, atau langsung jelajahi katalog." },
];
const last = STEPS.length - 1;
const dialog = ref<HTMLDivElement>();

function close() {
  open.value = false;
  try { localStorage.setItem(KEY, "1"); } catch {}
}
function finish(action: "folder" | "catalog") {
  close();
  emit("done", action);
}
// "**bold** rest" in a note item, escaped first since it goes through v-html
const bold = (s: string) => s.replace(/[&<>]/g, (c) => `&#${c.charCodeAt(0)};`).replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
const releaseUrl = (v: string) => `https://github.com/okuru-id/neptunus/releases/tag/v${v}`;
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") close();
  else if (news.value) return;
  else if (e.key === "ArrowRight" && step.value < last) step.value++;
  else if (e.key === "ArrowLeft" && step.value > 0) step.value--;
}
watch(open, async (o) => o && (await nextTick(), dialog.value?.focus()));
</script>

<template>
  <Transition name="dialog">
    <div v-if="open" class="scrim" @keydown="onKey">
      <div ref="dialog" class="dialog" role="dialog" aria-modal="true" aria-labelledby="ob-title" aria-describedby="ob-text" tabindex="-1">
        <template v-if="news">
          <span class="badge" :style="{ clipPath: 'url(#shape-sunny8)' }"><Icon name="update" :size="44" /></span>
          <h2 id="ob-title">Yang baru di v{{ news.version }}</h2>
          <p id="ob-text">{{ news.summary || "Neptunus sudah diperbarui." }}</p>
          <ul v-if="news.items.length" class="notes">
            <li v-for="(it, i) in news.items" :key="i" v-html="bold(it)"></li>
          </ul>
          <div class="actions">
            <button class="btn tonal" @click="openUrl(releaseUrl(news.version)).catch(() => {})"><Icon name="update" :size="20" />Catatan rilis</button>
            <button class="btn filled" @click="close">Oke</button>
          </div>
        </template>
        <template v-else>
        <button class="skip" @click="close">{{ step < last ? "Lewati" : "Tutup" }}</button>
        <Transition name="card" mode="out-in">
          <div :key="step" class="card">
            <span class="badge" :style="{ clipPath: `url(#shape-${STEPS[step].shape})` }"><Icon :name="STEPS[step].icon" :size="44" /></span>
            <h2 id="ob-title">{{ STEPS[step].title }}</h2>
            <p id="ob-text">{{ STEPS[step].text }}</p>
          </div>
        </Transition>
        <div class="dots" role="tablist" aria-label="Langkah">
          <button v-for="(s, i) in STEPS" :key="i" class="dot" :class="{ on: i === step, past: i < step }" role="tab" :aria-selected="i === step"
            :aria-label="`Langkah ${i + 1}`" @click="step = i"></button>
        </div>
        <div v-if="step < last" class="actions">
          <button class="btn tonal" :disabled="step === 0" @click="step--">Kembali</button>
          <button class="btn filled" @click="step++">Lanjut</button>
        </div>
        <div v-else class="actions">
          <button class="btn tonal" @click="finish('catalog')"><Icon name="catalog" :size="20" />Katalog</button>
          <button class="btn filled" @click="finish('folder')"><Icon name="addFolder" :size="20" />Pilih folder</button>
        </div>
        </template>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.scrim {
  position: fixed; inset: 0; z-index: 50; display: grid; place-items: center; padding: 24px;
  background: rgb(0 0 0 / 0.4); backdrop-filter: blur(6px);
}
.dialog {
  position: relative; overflow: hidden; outline: none;
  display: flex; flex-direction: column; align-items: center; text-align: center;
  width: min(440px, 100%); padding: 40px 28px 24px; border-radius: 32px;
  background:
    radial-gradient(120% 70% at 50% 0%, color-mix(in srgb, var(--primary) 16%, transparent), transparent 70%),
    var(--surface-container-high);
  color: var(--on-surface);
  box-shadow: 0 24px 64px rgb(0 0 0 / 0.35), inset 0 0 0 1px color-mix(in srgb, var(--on-surface) 6%, transparent);
}
.skip {
  position: absolute; top: 14px; right: 14px; height: 36px; padding: 0 16px; border-radius: 18px;
  font-size: 14px; font-weight: 600; color: var(--on-surface-variant);
  transition: background 0.2s var(--emph);
}
.skip:hover { background: color-mix(in srgb, var(--on-surface) 8%, transparent); }
.card { display: flex; flex-direction: column; align-items: center; min-height: 236px; }
.badge {
  width: 104px; height: 104px; display: grid; place-items: center; margin-bottom: 24px;
  background: var(--primary-container); color: var(--on-primary-container);
}
h2 {
  margin: 0 0 10px; font-size: 28px; line-height: 1.2; font-weight: 800; letter-spacing: -0.02em;
  font-variation-settings: "wdth" 115;
}
p { margin: 0; font-size: 15px; line-height: 1.55; color: var(--on-surface-variant); max-width: 36ch; }
.notes {
  align-self: stretch; margin: 20px 0 0; padding: 12px 16px 12px 32px; max-height: 40vh; overflow: auto;
  text-align: left; font-size: 14px; line-height: 1.5; color: var(--on-surface-variant);
  background: var(--surface-container); border-radius: 20px;
}
.notes li + li { margin-top: 8px; }
.notes :deep(strong) { color: var(--on-surface); font-weight: 650; }
/* steps done are primary, the current one grows into the app's cookie shape and pops in, the rest wait in outline */
.dots { display: flex; margin-top: 16px; }
.dot { display: grid; place-items: center; width: 28px; height: 28px; padding: 0; border-radius: 14px; background: none; }
.dot::before {
  content: ""; width: 16px; height: 16px; border-radius: 50%; background: var(--outline-variant);
  transform: scale(0.5); transition: transform 0.35s var(--spring), background 0.25s var(--emph);
}
.dot:hover::before { background: var(--on-surface-variant); }
.dot.past::before { background: color-mix(in srgb, var(--primary) 55%, transparent); }
.dot.on::before { background: var(--primary); clip-path: url(#shape-cookie9); transform: none; animation: dot-pop 0.5s var(--spring); }
@keyframes dot-pop { from { transform: scale(0.5) rotate(-120deg); } }
.dot:focus-visible { outline: none; box-shadow: 0 0 0 2px color-mix(in srgb, var(--primary) 45%, transparent); }
.actions { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; width: 100%; margin-top: 24px; }
.btn {
  display: inline-flex; align-items: center; justify-content: center; gap: 8px;
  height: 48px; padding: 0 20px; border-radius: 24px; font-size: 15px; font-weight: 600;
  transition: border-radius 0.3s var(--spring), filter 0.2s, box-shadow 0.2s, opacity 0.2s;
}
.btn:hover:not(:disabled) { filter: brightness(1.08); }
.btn:active:not(:disabled) { border-radius: 12px; }
.btn:disabled { opacity: 0.38; }
.btn:focus-visible { outline: none; box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 45%, transparent); }
.tonal { background: var(--surface-container-highest); color: var(--on-surface); }
.filled { background: var(--primary); color: var(--on-primary); }

.card-enter-active { transition: transform 0.4s var(--spring), opacity 0.25s var(--emph); }
.card-leave-active { transition: transform 0.15s var(--emph), opacity 0.15s var(--emph); }
.card-enter-from { transform: translateX(24px) scale(0.96); opacity: 0; }
.card-leave-to { transform: translateX(-16px); opacity: 0; }
.card-enter-active .badge { animation: pop 0.5s var(--spring); }
@keyframes pop { from { transform: scale(0.4) rotate(-40deg); } }

.dialog-enter-active, .dialog-leave-active { transition: opacity 0.25s var(--emph); }
.dialog-enter-active .dialog { transition: transform 0.45s var(--spring), opacity 0.25s; }
.dialog-leave-active .dialog { transition: transform 0.2s var(--emph), opacity 0.2s; }
.dialog-enter-from, .dialog-leave-to { opacity: 0; }
.dialog-enter-from .dialog { transform: translateY(24px) scale(0.92); opacity: 0; }
.dialog-leave-to .dialog { transform: scale(0.96); opacity: 0; }

@media (prefers-reduced-motion: reduce) {
  .card-enter-active, .card-leave-active { transition: opacity 0.15s; }
  .card-enter-active .badge, .dot.on::before { animation: none; }
}
</style>
