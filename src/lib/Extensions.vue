<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { computed, onMounted, reactive, ref, watch } from "vue";
import Icon from "./Icon.vue";
import { confirm } from "./ConfirmDialog.vue";
import { extName, has, newer, unsupported, type ExtInfo, type RepoExt } from "./ext";
import { rowCatalogs, sourceInfo, type SourceInfo } from "./videoext";
import { FEATURES, featureOn, loadFeatures, setFeature } from "./videoprefs";
import { pasted, verify, verifyError, waiting } from "./verify";

// Settings › Ekstensi: install from a repository (registry.json) or a package file, then
// enable, configure or remove. Emits `changed` whenever the installed set changes.
const emit = defineEmits<{ changed: [ExtInfo[]] }>();

const REPO_KEY = "neptunus.extRepo";
// Mornye (the successor of SpotiFLAC) keeps the extension registry; loaded when none was picked
const DEFAULT_REPO = "https://github.com/mornyeapp/Mornye-Extension";
const KIND_KEY = "neptunus.extKind";
const installed = ref<ExtInfo[]>([]);
const repoUrl = ref("");
const stremioUrl = ref("");
const repo = ref<RepoExt[] | null>(null);
const repoBusy = ref(false);
const busy = ref<string | null>(null); // id being installed / saved
const msg = ref<{ text: string; err?: boolean } | null>(null);
const openId = ref<string | null>(null);
const draft = ref<Record<string, unknown>>({});

const TYPE_LABEL: Record<string, string> = { metadata_provider: "Katalog", download_provider: "Unduh", lyrics_provider: "Lirik", video_provider: "Video" };
const byId = computed(() => new Map(installed.value.map((e) => [e.id, e])));

// Music (SpotiFLAC catalog/download/lyrics) and video (catalog addons) extensions are set up apart,
// so one list never mixes the two.
type Kind = "music" | "video";
const stored = (() => { try { return localStorage.getItem(KIND_KEY); } catch { return null; } })();
const kind = ref<Kind>(stored === "video" ? "video" : "music");
const isVideo = (e: ExtInfo) => has(e, "video_provider");
const count = computed(() => ({
  music: installed.value.filter((e) => !isVideo(e)).length,
  video: installed.value.filter(isVideo).length,
}));
const listed = computed(() => installed.value.filter((e) => isVideo(e) === (kind.value === "video")));
// music: the repository's extensions join the installed ones in one list, with an install button
const repoById = computed(() => new Map((repo.value ?? []).map((r) => [r.id, r])));
const available = computed(() => kind.value === "music" ? (repo.value ?? []).filter((r) => !byId.value.get(r.id)) : []);
const update = (e: ExtInfo) => {
  const r = repoById.value.get(e.id);
  return r && newer(r.version, e.version) ? r : undefined;
};
// video: which of catalog / sources / subtitles each addon offers, so only those get a switch
const vinfo = reactive<Record<string, SourceInfo | null>>({});
loadFeatures();
watch(installed, (list) => {
  for (const e of list.filter(isVideo)) {
    if (e.id in vinfo) continue;
    vinfo[e.id] = null;
    sourceInfo(e.id).then((i) => (vinfo[e.id] = i), () => {});
  }
});
const offers = (e: ExtInfo) => {
  const i = vinfo[e.id];
  if (!i) return FEATURES; // not known yet: show them all
  return FEATURES.filter((f) => f.id === "catalog" ? rowCatalogs(i).length : f.id === "streams" ? i.streams !== false : i.subtitles);
};

// signed sessions (some SpotiFLAC download extensions): whether the gateway's browser check still holds,
// with a button to do it (again). Downloads don't open the page themselves; they point here.
type Session = { verified: boolean; expires_at?: string | null };
const sessions = reactive<Record<string, Session>>({});
const verifyFailed = reactive<Record<string, string>>({});
const mobile = document.documentElement.classList.contains("mobile");
async function loadSessions(list: ExtInfo[]) {
  for (const e of list.filter((e) => e.signedSession)) {
    sessions[e.id] = await invoke<Session>("ext_session_status", { id: e.id }).catch(() => ({ verified: false }));
  }
}
watch(installed, loadSessions);
async function startVerify(e: ExtInfo) {
  delete verifyFailed[e.id];
  try {
    // on phones the page replaces the app, which finishes the verification when it comes back
    if (!(await verify(e.id)) && verifyError.value) verifyFailed[e.id] = verifyError.value;
  } catch (err) {
    verifyFailed[e.id] = String(err);
  }
  await loadSessions([e]);
}
const until = (s?: string | null) => {
  const d = s ? new Date(s) : null;
  return d && !isNaN(+d) ? d.toLocaleString("id-ID", { dateStyle: "medium", timeStyle: "short" }) : "";
};

function setKind(k: Kind) {
  kind.value = k;
  openId.value = null;
  try { localStorage.setItem(KIND_KEY, k); } catch {}
}

function flash(text: string, err = false) {
  msg.value = { text, err };
  if (!err) setTimeout(() => msg.value?.text === text && (msg.value = null), 4000);
}
async function reload() {
  installed.value = await invoke<ExtInfo[]>("ext_list");
  emit("changed", installed.value);
}
// the field empties once loaded, like the video one; the URL stays saved and reloads on open
async function loadRepo(url = repoUrl.value.trim()) {
  if (!url) return;
  repoBusy.value = true;
  try {
    const reg = await invoke<{ extensions: RepoExt[] }>("ext_repo", { url });
    repo.value = reg.extensions;
    localStorage.setItem(REPO_KEY, url);
    repoUrl.value = "";
  } catch (e) {
    repo.value = null;
    flash(`Gagal memuat repository: ${e}`, true);
  } finally {
    repoBusy.value = false;
  }
}
async function install(r: RepoExt) {
  busy.value = r.id;
  try {
    const e = await invoke<ExtInfo>("ext_install_url", { url: r.download_url || r.downloadUrl, sha256: r.sha256 || r.checksum_sha256 || null });
    await reload();
    flash(`${extName(e)} ${e.version} terpasang.`);
  } catch (e) {
    flash(`Gagal memasang: ${e}`, true);
  } finally {
    busy.value = null;
  }
}
async function installFile() {
  const path = await open({ filters: [{ name: "Ekstensi", extensions: ["spotiflac-ext", "sflx", "zip"] }] });
  if (typeof path !== "string") return;
  busy.value = "file";
  try {
    const e = await invoke<ExtInfo>("ext_install_file", { path });
    await reload();
    flash(`${extName(e)} ${e.version} terpasang.`);
  } catch (e) {
    flash(`Gagal memasang: ${e}`, true);
  } finally {
    busy.value = null;
  }
}
// a Stremio addon is just a manifest URL — the app wraps it into a video_provider extension
async function installStremio() {
  busy.value = "stremio";
  try {
    const e = await invoke<ExtInfo>("ext_install_stremio", { url: stremioUrl.value.trim() });
    stremioUrl.value = "";
    await reload();
    flash(`Addon ${extName(e)} terpasang. Buka menu Video untuk memakainya.`);
  } catch (e) {
    flash(`Gagal memasang addon: ${e}`, true);
  } finally {
    busy.value = null;
  }
}
async function toggle(e: ExtInfo) {
  await invoke("ext_set_enabled", { id: e.id, enabled: !e.enabled });
  await reload();
}
async function remove(e: ExtInfo) {
  const ok = await confirm({ title: `Hapus ${extName(e)}?`, message: "Ekstensi, pengaturan dan datanya dihapus dari aplikasi ini.", confirm: "Hapus", icon: "trash", danger: true });
  if (!ok) return;
  await invoke("ext_remove", { id: e.id });
  if (openId.value === e.id) openId.value = null;
  await reload();
}
function edit(e: ExtInfo) {
  openId.value = openId.value === e.id ? null : e.id;
  draft.value = { ...e.values };
}
async function save(e: ExtInfo) {
  busy.value = e.id;
  try {
    await invoke("ext_save_settings", { id: e.id, values: draft.value });
    await reload();
    flash("Pengaturan disimpan.");
  } finally {
    busy.value = null;
  }
}
// a "button" setting runs an exported function (login flows and the like)
async function action(e: ExtInfo, fn: string) {
  busy.value = e.id;
  try {
    await invoke("ext_save_settings", { id: e.id, values: draft.value });
    const res = await invoke<{ message?: string; error?: string } | null>("ext_call", { id: e.id, method: fn, args: [] });
    flash(res?.error ? `Gagal: ${res.error}` : res?.message || "Selesai.", !!res?.error);
    await reload();
  } catch (err) {
    flash(`Gagal: ${err}`, true);
  } finally {
    busy.value = null;
  }
}

onMounted(() => {
  reload();
  loadRepo(localStorage.getItem(REPO_KEY) || DEFAULT_REPO);
});
</script>

<template>
  <div class="exts">
    <div class="kinds" role="tablist" aria-label="Jenis ekstensi">
      <button role="tab" :aria-selected="kind === 'music'" :class="{ on: kind === 'music' }" @click="setKind('music')">
        <Icon name="note" :size="20" />Musik<span v-if="count.music" class="n">{{ count.music }}</span>
      </button>
      <button role="tab" :aria-selected="kind === 'video'" :class="{ on: kind === 'video' }" @click="setKind('video')">
        <Icon name="movie" :size="20" />Video<span v-if="count.video" class="n">{{ count.video }}</span>
      </button>
    </div>

    <section class="panel">
      <div class="head">
        <span class="mark" :class="{ alt: kind === 'video' }" :style="{ clipPath: 'url(#shape-cookie9)' }"><Icon :name="kind === 'video' ? 'movie' : 'note'" :size="28" /></span>
        <div class="id">
          <strong>{{ kind === "video" ? "Ekstensi video" : "Ekstensi musik" }}</strong>
          <span v-if="kind === 'video'">Katalog film dan serial untuk menu Video, dari URL manifest addon (format Stremio).</span>
          <span v-else>Sumber katalog, unduhan dan lirik lagu. Format ekstensi Mornye / SpotiFLAC (.sflx, .spotiflac-ext).</span>
        </div>
        <button v-if="kind === 'music'" class="btn tonal" :disabled="busy === 'file'" @click="installFile"><Icon name="upload" />Pasang dari file</button>
      </div>
      <Transition name="fade">
        <p v-if="msg" class="status" :class="{ err: msg.err }" role="status">
          <span>{{ msg.text }}</span>
          <button class="icon-btn small" title="Tutup" @click="msg = null"><Icon name="close" :size="18" /></button>
        </p>
      </Transition>

      <form v-if="kind === 'music'" class="repo-form" @submit.prevent="loadRepo()">
        <input v-model="repoUrl" placeholder="URL repository ekstensi Mornye / SpotiFLAC (repo GitHub atau …/registry.json)" aria-label="URL repository" />
        <button class="btn filled" :disabled="repoBusy || !repoUrl.trim()"><Icon name="sync" />{{ repoBusy ? "Memuat…" : "Muat" }}</button>
      </form>
      <form v-else class="repo-form" @submit.prevent="installStremio">
        <input v-model="stremioUrl" placeholder="URL manifest addon Stremio (…/manifest.json)" aria-label="URL manifest addon Stremio" />
        <button class="btn filled" :disabled="busy === 'stremio' || !stremioUrl.trim()"><Icon name="download" />{{ busy === "stremio" ? "Memasang…" : "Tambah" }}</button>
      </form>

      <ul v-if="listed.length || available.length" class="list">
        <li v-for="e in listed" :key="e.id" class="item" :class="{ off: !e.enabled, open: openId === e.id }">
          <div class="item-row">
            <img v-if="e.iconData" :src="e.iconData" class="ic" alt="" />
            <span v-else class="ic ph"><Icon name="extension" /></span>
            <div class="meta">
              <strong>{{ extName(e) }} <small>{{ e.version }}</small></strong>
              <span class="tags">
                <span v-for="t in e.type.filter((t) => t !== 'video_provider')" :key="t" class="tag">{{ TYPE_LABEL[t] ?? t }}</span>
                <span v-if="unsupported(e).length" class="tag warn" :title="unsupported(e).join(', ')">Butuh fitur yang belum didukung</span>
              </span>
            </div>
            <div class="acts">
              <button v-if="update(e)" class="btn filled sm" :disabled="busy === e.id" @click="install(update(e)!)"><Icon name="sync" />{{ busy === e.id ? "Memperbarui…" : "Perbarui" }}</button>
              <button v-if="e.settings?.length" class="icon-btn" :class="{ on: openId === e.id }" title="Pengaturan" @click="edit(e)"><Icon name="settings" /></button>
              <button class="icon-btn" title="Hapus" @click="remove(e)"><Icon name="trash" /></button>
              <button class="switch" role="switch" :aria-checked="e.enabled" :title="e.enabled ? 'Nonaktifkan' : 'Aktifkan'" @click="toggle(e)"><span></span></button>
            </div>
          </div>
          <div v-if="e.signedSession && e.enabled" class="verif" :class="{ ok: sessions[e.id]?.verified }">
            <Icon :name="sessions[e.id]?.verified ? 'verified' : 'error'" :size="18" />
            <span class="v-text">
              <template v-if="waiting === e.id">Menunggu verifikasi selesai…</template>
              <template v-else-if="verifyFailed[e.id]">{{ verifyFailed[e.id] }}</template>
              <template v-else-if="!sessions[e.id]">Memeriksa sesi…</template>
              <template v-else-if="sessions[e.id].verified">Terverifikasi{{ until(sessions[e.id].expires_at) ? ` sampai ${until(sessions[e.id].expires_at)}` : "" }}</template>
              <template v-else>Perlu verifikasi sebelum bisa mengunduh</template>
            </span>
            <button class="btn sm" :class="sessions[e.id]?.verified ? 'tonal' : 'filled'" :disabled="!!waiting" @click="startVerify(e)">
              {{ sessions[e.id]?.verified ? "Verifikasi ulang" : "Verifikasi" }}
            </button>
            <div v-if="waiting === e.id && !mobile" class="paste">
              Selesaikan verifikasi di jendela yang terbuka. Kalau jendela tidak menutup sendiri, tempel tautan
              <code>spotiflac://session-grant?…</code> di sini:
              <input v-model="pasted" placeholder="spotiflac://session-grant?grant=…&state=…" spellcheck="false" />
            </div>
          </div>
          <div v-if="kind === 'video' && e.enabled && offers(e).length" class="feats" role="group" aria-label="Fitur yang dipakai">
            <button v-for="f in offers(e)" :key="f.id" type="button" role="switch" :aria-checked="featureOn(e.id, f.id)"
              :class="{ on: featureOn(e.id, f.id) }" @click="setFeature(e.id, f.id, !featureOn(e.id, f.id))">
              <Icon :name="featureOn(e.id, f.id) ? 'done' : 'close'" :size="16" />{{ f.label }}
            </button>
          </div>
          <form v-if="openId === e.id" class="form" @submit.prevent="save(e)">
            <p v-if="e.description" class="hint">{{ e.description }}</p>
            <template v-for="s in e.settings" :key="s.key">
              <button v-if="s.type === 'button'" type="button" class="btn tonal" :disabled="busy === e.id" @click="action(e, s.action || s.key)">{{ s.label }}</button>
              <label v-else-if="s.type === 'boolean'" class="check">
                <input type="checkbox" :checked="!!draft[s.key]" @change="draft[s.key] = ($event.target as HTMLInputElement).checked" />
                <span>{{ s.label }}<small v-if="s.description">{{ s.description }}</small></span>
              </label>
              <label v-else class="field">
                {{ s.label }}
                <select v-if="s.type === 'select'" v-model="draft[s.key]">
                  <option v-for="o in s.options" :key="o" :value="o">{{ o }}</option>
                </select>
                <input v-else :type="s.secret ? 'password' : s.type === 'number' ? 'number' : 'text'" autocomplete="off"
                  :value="draft[s.key] ?? ''" @input="draft[s.key] = s.type === 'number' ? Number(($event.target as HTMLInputElement).value) : ($event.target as HTMLInputElement).value" />
                <small v-if="s.description">{{ s.description }}</small>
              </label>
            </template>
            <div class="btn-group">
              <button class="btn filled" :disabled="busy === e.id"><Icon name="save" />Simpan</button>
            </div>
          </form>
        </li>
        <li v-for="r in available" :key="r.id" class="item">
          <div class="item-row">
            <img v-if="r.icon_url || r.iconUrl" :src="r.icon_url || r.iconUrl" class="ic" alt="" loading="lazy" />
            <span v-else class="ic ph"><Icon name="extension" /></span>
            <div class="meta">
              <strong>{{ r.display_name || r.displayName || r.name || r.id }} <small>{{ r.version }}</small></strong>
              <span class="desc">{{ r.description }}</span>
            </div>
            <div class="acts">
              <button class="btn tonal sm" :disabled="busy === r.id" @click="install(r)"><Icon name="download" />{{ busy === r.id ? "Memasang…" : "Pasang" }}</button>
            </div>
          </div>
        </li>
      </ul>
      <p v-else class="hint">{{ kind === "video" ? "Belum ada ekstensi video. Tempel URL manifest di atas untuk menambah katalog ke menu Video." : "Belum ada ekstensi musik. Muat repository di atas, atau pasang dari file." }}</p>
    </section>

  </div>
</template>

<style scoped>
.exts { display: flex; flex-direction: column; gap: 16px; }
.feats { display: flex; flex-wrap: wrap; gap: 6px; padding: 0 12px 12px 68px; } /* lined up with the name */
.feats button {
  display: inline-flex; align-items: center; gap: 4px; height: 30px; padding: 0 12px 0 8px; border-radius: 15px; font-size: 13px; font-weight: 600;
  color: var(--on-surface-variant); border: 1px solid var(--outline-variant); transition: background 0.2s, color 0.2s, border-color 0.2s;
}
.feats button:hover { background: var(--surface-container-highest); }
.feats button.on { background: var(--secondary-container); color: var(--on-secondary-container); border-color: transparent; }
.panel { padding: 28px; border-radius: 28px; background: var(--surface-container-low); display: flex; flex-direction: column; gap: 20px; }
.head { display: flex; align-items: center; gap: 16px; flex-wrap: wrap; }
.mark { width: 56px; height: 56px; display: grid; place-items: center; flex: none; background: var(--primary-container); color: var(--on-primary-container); }
.mark.alt { background: var(--tertiary-container); color: var(--on-tertiary-container); }
.id { flex: 1; min-width: 200px; display: flex; flex-direction: column; gap: 2px; }
.id strong { font-size: 20px; font-weight: 700; }
.id span, .hint { color: var(--on-surface-variant); font-size: 14px; line-height: 1.5; margin: 0; }

.btn {
  display: inline-flex; align-items: center; gap: 8px; height: 48px; padding: 0 24px 0 18px; border-radius: 24px;
  font-size: 15px; font-weight: 600; transition: border-radius 0.3s var(--spring), background 0.2s; flex: none;
}
.btn.sm { height: 40px; padding: 0 18px 0 14px; border-radius: 20px; font-size: 14px; }
.btn:active { border-radius: 12px; }
.btn:disabled { opacity: 0.5; cursor: default; }
.btn-group { display: flex; flex-wrap: wrap; gap: 8px; }
.filled { background: var(--primary); color: var(--on-primary); }
.tonal { background: var(--secondary-container); color: var(--on-secondary-container); }
.icon-btn {
  width: 40px; height: 40px; border-radius: 20px; display: inline-grid; place-items: center; flex: none;
  color: var(--on-surface-variant); transition: background 0.2s, border-radius 0.3s var(--spring);
}
.icon-btn:hover { background: var(--surface-container-highest); }
.icon-btn.on { color: var(--on-primary-container); background: var(--primary-container); }
.icon-btn.small { width: 32px; height: 32px; color: inherit; margin: -6px -8px -6px 0; }

.status { display: flex; align-items: center; gap: 8px; margin: 0; padding: 12px 16px; border-radius: 12px; background: var(--secondary-container); color: var(--on-secondary-container); }
.status > span { flex: 1; overflow-wrap: anywhere; }
.status.err { background: #8c1d18; color: #ffdad6; }
.fade-enter-active, .fade-leave-active { transition: opacity 0.25s var(--emph); }
.fade-enter-from, .fade-leave-to { opacity: 0; }

.list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
.item { border-radius: 20px; transition: background 0.2s; }
.item:hover, .item.open { background: var(--surface-container); }
.item.off .ic, .item.off .meta { opacity: 0.5; }
.item-row { display: flex; align-items: center; gap: 12px; padding: 10px 12px; min-height: 64px; }
.ic { width: 44px; height: 44px; border-radius: 12px; object-fit: cover; flex: none; }
.ic.ph { display: grid; place-items: center; background: var(--secondary-container); color: var(--on-secondary-container); }
.meta { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
.meta strong { font-size: 16px; font-weight: 650; }
.meta small { font-size: 13px; font-weight: 500; color: var(--on-surface-variant); margin-left: 4px; }
.desc { font-size: 13px; color: var(--on-surface-variant); overflow: hidden; text-overflow: ellipsis; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; }
.tags { display: flex; flex-wrap: wrap; gap: 6px; }
.tag { height: 22px; padding: 0 8px; border-radius: 6px; display: inline-flex; align-items: center; font-size: 12px; font-weight: 600; background: var(--surface-container-highest); color: var(--on-surface-variant); }
.tag.warn { background: color-mix(in srgb, #ffb4ab 22%, transparent); color: var(--on-surface); }
.ok { display: inline-flex; align-items: center; gap: 6px; font-size: 14px; font-weight: 600; color: var(--primary); padding: 0 8px; flex: none; }

/* M3 switch */
.switch { width: 52px; height: 32px; border-radius: 16px; flex: none; position: relative; background: var(--surface-container-highest); box-shadow: inset 0 0 0 2px var(--on-surface-variant); transition: background 0.2s; }
.switch span { position: absolute; top: 8px; left: 8px; width: 16px; height: 16px; border-radius: 50%; background: var(--on-surface-variant); transition: all 0.3s var(--spring); }
.switch[aria-checked="true"] { background: var(--primary); box-shadow: none; }
.switch[aria-checked="true"] span { top: 4px; left: 24px; width: 24px; height: 24px; background: var(--on-primary); }

.form { display: flex; flex-direction: column; gap: 16px; padding: 4px 16px 20px 68px; }
.field { display: flex; flex-direction: column; gap: 6px; font-size: 13px; font-weight: 600; color: var(--on-surface-variant); max-width: 480px; }
.field small, .check small { display: block; font-weight: 400; font-size: 12px; }
.field input, .field select, .repo-form input {
  height: 48px; padding: 0 16px; border: 0; border-radius: 12px 12px 4px 4px; outline: none;
  background: var(--surface-container-highest); color: var(--on-surface); font-size: 15px; font-weight: 400;
  box-shadow: inset 0 -1px 0 var(--on-surface-variant); transition: box-shadow 0.2s;
}
.field input:focus, .field select:focus, .repo-form input:focus { box-shadow: inset 0 -3px 0 var(--primary); }
.check { display: flex; align-items: flex-start; gap: 12px; font-size: 15px; }
.check input { width: 18px; height: 18px; margin-top: 2px; accent-color: var(--primary); }
.repo-form { display: flex; gap: 8px; }
.repo-form input { flex: 1; min-width: 0; }
.kinds { display: flex; gap: 4px; padding: 4px; border-radius: 28px; background: var(--surface-container-low); align-self: flex-start; }
.kinds button {
  height: 48px; padding: 0 20px 0 16px; border-radius: 24px; display: inline-flex; align-items: center; gap: 8px;
  font-size: 15px; font-weight: 600; color: var(--on-surface-variant); transition: background 0.2s, color 0.2s;
}
.kinds button:hover { background: var(--surface-container-highest); }
.kinds button.on { background: var(--secondary-container); color: var(--on-secondary-container); }
.kinds .n { min-width: 22px; height: 22px; padding: 0 6px; border-radius: 11px; display: inline-grid; place-items: center; font-size: 12px; background: var(--surface-container-highest); }
.kinds button.on .n { background: var(--on-secondary-container); color: var(--secondary-container); }
.acts { display: flex; align-items: center; gap: 8px; flex: none; }
.verif { display: flex; align-items: center; flex-wrap: wrap; gap: 8px 10px; margin: 0 12px 10px; padding: 8px 8px 8px 12px; border-radius: 14px; background: var(--surface-container-high); font-size: 13px; color: var(--on-surface-variant); }
.verif > .icon { color: var(--error); }
.verif.ok > .icon { color: var(--primary); }
.v-text { flex: 1; min-width: 140px; }
.paste { flex-basis: 100%; display: flex; flex-direction: column; gap: 8px; line-height: 1.5; }
.paste input { height: 40px; padding: 0 12px; border: 1px solid var(--outline-variant); border-radius: 12px; background: var(--surface-container-lowest); color: var(--on-surface); font: inherit; }
@media (max-width: 600px) {
  .form { padding-left: 16px; }
  /* phones: the name gets the whole row, the buttons go on a row of their own below it */
  .panel { padding: 16px 8px; border-radius: 24px; gap: 16px; }
  .item-row { flex-wrap: wrap; row-gap: 2px; gap: 10px; padding: 8px; min-height: 0; }
  .ic { width: 40px; height: 40px; }
  .meta { flex-basis: calc(100% - 50px); }
  .acts { flex-basis: 100%; justify-content: flex-end; }
  .verif { margin: 0 8px 8px; }
}
</style>
