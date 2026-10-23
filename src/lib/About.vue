<script setup lang="ts">
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { onMounted, onUnmounted, reactive, ref } from "vue";
import Icon from "./Icon.vue";
import { installUpdate, openApk, updater } from "./updater";
import license from "../../LICENSE?raw";

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
// "Semua rilis": the releases and their files right here, each downloadable without a browser
type Release = { tag: string; date: string; assets: [string, number, string][] };
const showReleases = ref(false);
const releases = ref<Release[] | null>(null);
const releaseError = ref("");
const openTag = ref("");
const dl = reactive<Record<string, { pct: number; done?: string; error?: string }>>({});
const mb = (b: number) => (b >= 1 << 30 ? `${(b / (1 << 30)).toFixed(1)} GB` : `${(b / (1 << 20)).toFixed(1)} MB`);
async function toggleReleases() {
  showReleases.value = !showReleases.value;
  if (!showReleases.value || releases.value) return;
  releaseError.value = "";
  try {
    releases.value = await invoke<Release[]>("release_list");
    openTag.value = releases.value[0]?.tag ?? "";
  } catch (e) {
    releaseError.value = String(e);
  }
}
async function download(name: string, url: string) {
  dl[url] = { pct: 0 };
  try {
    dl[url].done = openApk(await invoke<string>("release_download", { url, name }));
  } catch (e) {
    dl[url].error = String(e);
  }
}
let offDl: (() => void) | undefined;
listen<{ key: string; pct: number }>("release-download", ({ payload }) => {
  if (dl[payload.key] && !dl[payload.key].done) dl[payload.key].pct = payload.pct;
}).then((f) => (offDl = f));
onUnmounted(() => offDl?.());

const showLicense = ref(false);
// a failed open (no browser handler, blocked URL) used to vanish silently: say what happened
const linkError = ref("");
async function openLink(url: string) {
  linkError.value = "";
  try {
    await openUrl(url);
  } catch (e) {
    linkError.value = `Link tidak bisa dibuka (${e}). Alamatnya: ${url}`;
  }
}
const mobile = document.documentElement.classList.contains("mobile");

// iOS: how long the signature this copy was installed with still lasts
const signing = ref<{ until: Date; days: number; team: string } | null>(null);
invoke<[string, string] | null>("ios_signing").then((r) => {
  if (!r) return;
  const until = new Date(r[0]);
  signing.value = { until, days: Math.ceil((until.getTime() - Date.now()) / 86400000), team: r[1] };
}, () => {});

const LINKS = [
  { label: "Situs web", url: "https://neptunus.id", icon: "home" },
  { label: "Kode sumber", url: REPO, icon: "code" },
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
        <template v-else-if="state.kind === 'new'">
          <span v-if="updater.busy">Mengunduh installer… {{ updater.pct ? `${updater.pct}%` : "" }}</span>
          <span v-else-if="updater.error">{{ updater.error }}</span>
          <span v-else-if="updater.done">{{ updater.done === "ota" ? "Ketuk Pasang di dialog iOS; pembaruan berjalan di layar Utama." : updater.done === "apk" ? "Ketuk Perbarui di dialog Android." : updater.done === "apk-permission" ? "Izinkan Neptunus memasang aplikasi, lalu ketuk Unduh & pasang lagi." : mobile ? "Dibuka di browser untuk diunduh." : `Tersimpan di ${updater.done}, pemasangan dibuka.` }}</span>
          <span v-else>{{ mobile ? "Unduh installer langsung." : "Unduh installer dan pasang langsung dari sini." }}</span>
          <span v-if="updater.busy" class="bar" :style="{ '--pct': `${updater.pct}%` }"></span>
        </template>
        <span v-else>Periksa rilis terbaru di GitHub.</span>
      </div>
      <button v-if="state.kind === 'new'" class="act filled" :disabled="updater.busy" @click="installUpdate"><Icon name="download" />{{ updater.busy ? "Mengunduh…" : updater.done ? "Unduh lagi" : "Unduh & pasang" }}</button>
      <button v-else class="act tonal" :disabled="state.kind === 'busy'" @click="check"><Icon name="sync" />Cek update</button>
    </div>

    <div v-if="signing" class="update sign" :class="{ warn: signing.days <= 2 }">
      <span class="u-icon"><Icon :name="signing.days > 2 ? 'done' : 'error'" :size="22" /></span>
      <div class="u-text">
        <strong>{{ signing.days > 0 ? `Sign aktif ${signing.days} hari lagi` : "Masa sign sudah habis" }}</strong>
        <span>Berlaku sampai {{ signing.until.toLocaleString("id-ID", { dateStyle: "long", timeStyle: "short" }) }}{{ signing.team ? ` · ${signing.team}` : "" }}. Perbarui (refresh) lewat SideStore atau iloader sebelum habis.</span>
      </div>
    </div>

    <div class="links">
      <button v-for="l in LINKS" :key="l.url" class="link" @click="openLink(l.url)">
        <Icon :name="l.icon" :size="20" /><span class="label">{{ l.label }}</span><Icon name="chevron" :size="20" class="go" />
      </button>
      <button class="link" :aria-expanded="showReleases" @click="toggleReleases">
        <Icon name="update" :size="20" /><span class="label">Semua rilis</span><Icon name="chevron" :size="20" class="go" :class="{ down: showReleases }" />
      </button>
      <div v-if="showReleases" class="releases">
        <p v-if="releaseError" class="rnote">Gagal memuat rilis ({{ releaseError }}).</p>
        <p v-else-if="!releases" class="rnote">Memuat daftar rilis…</p>
        <div v-for="r in releases ?? []" :key="r.tag" class="rel" :class="{ open: openTag === r.tag }">
          <button class="rhead" @click="openTag = openTag === r.tag ? '' : r.tag">
            <strong>{{ r.tag }}</strong>
            <span>{{ r.date ? new Date(r.date).toLocaleDateString("id-ID", { dateStyle: "medium" }) : "" }} · {{ r.assets.length }} file</span>
            <Icon name="chevron" :size="18" class="go" :class="{ down: openTag === r.tag }" />
          </button>
          <ul v-if="openTag === r.tag" class="assets">
            <li v-for="[name, size, url] in r.assets" :key="url">
              <span class="aname" :title="name">{{ name }}</span>
              <span class="asize">{{ mb(size) }}</span>
              <span v-if="dl[url] && !dl[url].done && !dl[url].error" class="apct">{{ dl[url].pct }}%</span>
              <button class="adl" :class="{ ok: dl[url]?.done }" :disabled="!!dl[url] && !dl[url].done && !dl[url].error"
                :title="dl[url]?.error ?? dl[url]?.done ?? 'Unduh ke folder Downloads'" @click="download(name, url)">
                <Icon :name="dl[url]?.done ? 'downloadDone' : dl[url]?.error ? 'error' : 'download'" :size="20" />
              </button>
              <span v-if="dl[url] && !dl[url].done && !dl[url].error" class="abar" :style="{ '--pct': `${dl[url].pct}%` }"></span>
              <span v-if="dl[url]?.error" class="aerr">{{ dl[url].error }}</span>
            </li>
          </ul>
        </div>
      </div>
      <button class="link" :aria-expanded="showLicense" @click="showLicense = !showLicense">
        <Icon name="info" :size="20" /><span class="label">Lisensi (MIT)</span><Icon name="chevron" :size="20" class="go" :class="{ down: showLicense }" />
      </button>
      <pre v-if="showLicense" class="license">{{ license }}</pre>
    </div>
    <p v-if="linkError" class="link-error">{{ linkError }}</p>

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
.go { color: var(--on-surface-variant); transition: transform 0.25s var(--spring); }
.bar { display: block; height: 4px; margin-top: 6px; border-radius: 2px; background: linear-gradient(to right, currentColor var(--pct), rgb(128 128 128 / 0.3) var(--pct)); }
.update.sign.warn { background: var(--error-container, #f9dedc); color: var(--on-error-container, #410e0b); }
.update.sign.warn .u-text, .update.sign.warn .u-text strong { color: inherit; }
.go.down { transform: rotate(90deg); }
.license {
  margin: 4px 0 0; padding: 16px; border-radius: 16px; max-height: 320px; overflow: auto; white-space: pre-wrap;
  font: 12.5px/1.55 ui-monospace, "SF Mono", Menlo, Consolas, monospace; color: var(--on-surface-variant);
  background: var(--surface-container); user-select: text;
}
.link-error { margin: 0; font-size: 13px; color: var(--on-surface-variant); user-select: text; word-break: break-all; }
.releases { display: flex; flex-direction: column; gap: 6px; padding: 4px 8px 10px; box-shadow: inset 0 1px 0 var(--outline-variant); }
.rnote { margin: 8px; font-size: 14px; color: var(--on-surface-variant); }
.rel { border-radius: 14px; }
.rel.open { background: var(--surface-container-high); }
.rhead { width: 100%; display: flex; align-items: center; gap: 10px; padding: 10px 12px; border-radius: 14px; text-align: left; color: var(--on-surface); }
.rhead:hover { background: var(--surface-container-highest); }
.rhead strong { font-size: 15px; }
.rhead span { flex: 1; font-size: 13px; color: var(--on-surface-variant); }
.assets { list-style: none; margin: 0; padding: 0 6px 8px; display: flex; flex-direction: column; }
.assets li { position: relative; display: flex; align-items: center; gap: 10px; flex-wrap: wrap; padding: 4px 6px 4px 8px; border-radius: 10px; }
.assets li:hover { background: var(--surface-container-highest); }
.aname { flex: 1; min-width: 0; font-size: 14px; font-family: ui-monospace, Menlo, Consolas, monospace; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.asize, .apct { font-size: 12px; color: var(--on-surface-variant); font-variant-numeric: tabular-nums; }
.adl { width: 36px; height: 36px; border-radius: 18px; display: grid; place-items: center; color: var(--on-secondary-container); background: var(--secondary-container); }
.adl:hover:not(:disabled) { background: var(--primary); color: var(--on-primary); }
.adl:disabled { opacity: 0.6; }
.adl.ok { background: transparent; color: var(--primary); }
.abar { flex-basis: 100%; height: 3px; border-radius: 2px; background: linear-gradient(to right, var(--primary) var(--pct), var(--surface-container-highest) var(--pct)); }
.aerr { flex-basis: 100%; font-size: 12px; color: var(--on-surface-variant); }
</style>
