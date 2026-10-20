<script setup lang="ts">
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { onMounted, ref } from "vue";
import Icon from "./Icon.vue";

// Settings › Tentang: app version, a manual update check (same GitHub lookup as the startup notice) and links.
const REPO = "https://github.com/okuru-id/neptunus";
const version = ref("");
type State = { kind: "idle" | "busy" | "latest" | "error"; text?: string } | { kind: "new"; version: string; url: string };
const state = ref<State>({ kind: "idle" });

async function check() {
  state.value = { kind: "busy" };
  try {
    const r = await invoke<[string, string] | null>("check_update");
    state.value = r ? { kind: "new", version: r[0], url: r[1] } : { kind: "latest" };
  } catch (e) {
    state.value = { kind: "error", text: String(e) };
  }
}
onMounted(async () => (version.value = await getVersion().catch(() => "")));

const LINKS = [
  { label: "Situs web", url: "https://neptunus.id", icon: "home" },
  { label: "Kode sumber", url: REPO, icon: "code" },
  { label: "Semua rilis", url: `${REPO}/releases`, icon: "update" },
] as const;
</script>

<template>
  <section class="panel about">
    <div class="head">
      <span class="mark" :style="{ clipPath: 'url(#shape-cookie9)' }"><Icon name="note" :size="32" /></span>
      <div class="id">
        <strong>Neptunus</strong>
        <span>Versi {{ version || "…" }}</span>
      </div>
    </div>
    <p class="hint">Pemutar musik desktop yang ringan untuk Linux, Windows dan macOS, dibuat dengan Tauri, Rust dan Vue.</p>

    <div class="update" :class="state.kind">
      <span class="u-icon"><Icon :name="state.kind === 'new' ? 'update' : state.kind === 'latest' ? 'done' : state.kind === 'error' ? 'error' : 'sync'" :size="22" /></span>
      <div class="u-text">
        <strong v-if="state.kind === 'new'">Versi {{ state.version }} tersedia</strong>
        <strong v-else-if="state.kind === 'latest'">Sudah versi terbaru</strong>
        <strong v-else-if="state.kind === 'busy'">Memeriksa…</strong>
        <strong v-else-if="state.kind === 'error'">Gagal memeriksa update</strong>
        <strong v-else>Pembaruan</strong>
        <span v-if="state.kind === 'error'">{{ state.text }}</span>
        <span v-else-if="state.kind === 'new'">Unduh rilis baru dari GitHub.</span>
        <span v-else>Periksa rilis terbaru di GitHub.</span>
      </div>
      <button v-if="state.kind === 'new'" class="act filled" @click="openUrl(state.url)"><Icon name="download" />Unduh</button>
      <button v-else class="act tonal" :disabled="state.kind === 'busy'" @click="check"><Icon name="sync" />Cek update</button>
    </div>

    <div class="links">
      <button v-for="l in LINKS" :key="l.url" class="link" @click="openUrl(l.url)">
        <Icon :name="l.icon" :size="20" /><span class="label">{{ l.label }}</span><Icon name="chevron" :size="20" class="go" />
      </button>
    </div>

    <p class="fine">
      Terinspirasi dari PixelPlayer untuk Android; proyek independen yang tidak berafiliasi. Lisensi MIT.
    </p>
  </section>
</template>

<style scoped>
.panel.about { max-width: none; }
.head { display: flex; align-items: center; gap: 16px; }
.mark { width: 64px; height: 64px; flex: none; display: grid; place-items: center; background: var(--tertiary-container); color: var(--on-tertiary-container); }
.id { display: flex; flex-direction: column; gap: 2px; }
.id strong { font-size: 28px; font-weight: 800; font-variation-settings: "wdth" 115; letter-spacing: -0.02em; }
.id span { color: var(--on-surface-variant); font-size: 14px; font-variant-numeric: tabular-nums; }
.hint { margin: 0; color: var(--on-surface-variant); line-height: 1.5; }

.update { display: flex; align-items: center; gap: 14px; padding: 14px 14px 14px 16px; border-radius: 20px; background: var(--surface-container); flex-wrap: wrap; }
.update.new { background: var(--primary-container); color: var(--on-primary-container); }
.u-icon { width: 40px; height: 40px; flex: none; display: grid; place-items: center; border-radius: 14px; background: var(--surface-container-highest); color: var(--on-surface-variant); }
.update.new .u-icon { background: var(--primary); color: var(--on-primary); }
.update.latest .u-icon { color: var(--primary); }
.u-text { flex: 1; min-width: 180px; display: flex; flex-direction: column; gap: 2px; font-size: 13px; color: var(--on-surface-variant); }
.update.new .u-text { color: inherit; opacity: 0.85; }
.u-text strong { font-size: 15px; font-weight: 700; color: var(--on-surface); }
.update.new .u-text strong { color: inherit; }

/* App.vue's .btn is scoped there, so the buttons are styled here (same M3 pill, squish on press) */
.act {
  display: inline-flex; align-items: center; gap: 8px; height: 40px; padding: 0 20px 0 16px; border-radius: 20px; flex: none;
  font-size: 14px; font-weight: 600; transition: border-radius 0.3s var(--spring), background-color 0.2s var(--emph);
}
.act:active { border-radius: 12px; }
.act:disabled { opacity: 0.5; cursor: default; }
.act.tonal { background: var(--secondary-container); color: var(--on-secondary-container); }
.act.filled { background: var(--primary); color: var(--on-primary); }
.links { display: flex; flex-direction: column; border-radius: 20px; overflow: hidden; background: var(--surface-container); }
.link {
  display: flex; align-items: center; gap: 14px; height: 52px; padding: 0 16px; text-align: left; font: inherit; font-size: 15px; font-weight: 500;
  color: var(--on-surface); background: transparent; transition: background-color 0.2s var(--emph);
}
.link + .link { box-shadow: inset 0 1px 0 var(--outline-variant); }
.link:hover { background: var(--surface-container-highest); }
.link > :first-child { color: var(--on-surface-variant); }
.link .label { flex: 1; }
.go { color: var(--on-surface-variant); }
.fine { margin: 0; font-size: 12px; color: var(--on-surface-variant); line-height: 1.5; }
</style>
