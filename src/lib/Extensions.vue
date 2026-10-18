<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { computed, onMounted, ref } from "vue";
import Icon from "./Icon.vue";
import { confirm } from "./ConfirmDialog.vue";
import { extName, has, newer, unsupported, type ExtInfo, type RepoExt } from "./ext";

// Settings › Ekstensi: install from a repository (registry.json) or a package file, then
// enable, configure or remove. Emits `changed` whenever the installed set changes.
const emit = defineEmits<{ changed: [ExtInfo[]] }>();

const REPO_KEY = "neptunus.extRepo";
const installed = ref<ExtInfo[]>([]);
const repoUrl = ref(localStorage.getItem(REPO_KEY) ?? "");
const repo = ref<RepoExt[] | null>(null);
const repoBusy = ref(false);
const busy = ref<string | null>(null); // id being installed / saved
const msg = ref<{ text: string; err?: boolean } | null>(null);
const openId = ref<string | null>(null);
const draft = ref<Record<string, unknown>>({});

const TYPE_LABEL: Record<string, string> = { metadata_provider: "Katalog", download_provider: "Unduh", lyrics_provider: "Lirik" };
const byId = computed(() => new Map(installed.value.map((e) => [e.id, e])));

function flash(text: string, err = false) {
  msg.value = { text, err };
  if (!err) setTimeout(() => msg.value?.text === text && (msg.value = null), 4000);
}
async function reload() {
  installed.value = await invoke<ExtInfo[]>("ext_list");
  emit("changed", installed.value);
}
async function loadRepo() {
  const url = repoUrl.value.trim();
  if (!url) return;
  repoBusy.value = true;
  try {
    const reg = await invoke<{ extensions: RepoExt[] }>("ext_repo", { url });
    repo.value = reg.extensions;
    localStorage.setItem(REPO_KEY, url);
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
  if (repoUrl.value) loadRepo();
});
</script>

<template>
  <div class="exts">
    <section class="panel">
      <div class="head">
        <span class="mark" :style="{ clipPath: 'url(#shape-cookie9)' }"><Icon name="extension" :size="28" /></span>
        <div class="id">
          <strong>Ekstensi</strong>
          <span>Tambah sumber katalog, unduhan dan lirik. Format ekstensi SpotiFLAC (.spotiflac-ext).</span>
        </div>
        <button class="btn tonal" :disabled="busy === 'file'" @click="installFile"><Icon name="upload" />Pasang dari file</button>
      </div>
      <Transition name="fade">
        <p v-if="msg" class="status" :class="{ err: msg.err }" role="status">
          <span>{{ msg.text }}</span>
          <button class="icon-btn small" title="Tutup" @click="msg = null"><Icon name="close" :size="18" /></button>
        </p>
      </Transition>

      <ul v-if="installed.length" class="list">
        <li v-for="e in installed" :key="e.id" class="item" :class="{ off: !e.enabled, open: openId === e.id }">
          <div class="item-row">
            <img v-if="e.iconData" :src="e.iconData" class="ic" alt="" />
            <span v-else class="ic ph"><Icon name="extension" /></span>
            <div class="meta">
              <strong>{{ extName(e) }} <small>{{ e.version }}</small></strong>
              <span class="tags">
                <span v-for="t in e.type" :key="t" class="tag">{{ TYPE_LABEL[t] ?? t }}</span>
                <span v-if="unsupported(e).length" class="tag warn" :title="unsupported(e).join(', ')">Butuh fitur yang belum didukung</span>
              </span>
            </div>
            <button v-if="e.settings?.length" class="icon-btn" :class="{ on: openId === e.id }" title="Pengaturan" @click="edit(e)"><Icon name="settings" /></button>
            <button class="icon-btn" title="Hapus" @click="remove(e)"><Icon name="trash" /></button>
            <button class="switch" role="switch" :aria-checked="e.enabled" :title="e.enabled ? 'Nonaktifkan' : 'Aktifkan'" @click="toggle(e)"><span></span></button>
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
      </ul>
      <p v-else class="hint">Belum ada ekstensi terpasang.</p>
    </section>

    <section class="panel">
      <div class="head">
        <span class="mark alt" :style="{ clipPath: 'url(#shape-cookie9)' }"><Icon name="store" :size="28" /></span>
        <div class="id">
          <strong>Repository</strong>
          <span>URL repository ekstensi (registry.json, atau repo GitHub yang berisi registry.json).</span>
        </div>
      </div>
      <form class="repo-form" @submit.prevent="loadRepo">
        <input v-model="repoUrl" placeholder="https://github.com/pemilik/repo-ekstensi" aria-label="URL repository" />
        <button class="btn filled" :disabled="repoBusy || !repoUrl.trim()"><Icon name="sync" />{{ repoBusy ? "Memuat…" : "Muat" }}</button>
      </form>
      <ul v-if="repo" class="list">
        <li v-for="r in repo" :key="r.id" class="item">
          <div class="item-row">
            <img v-if="r.icon_url || r.iconUrl" :src="r.icon_url || r.iconUrl" class="ic" alt="" loading="lazy" />
            <span v-else class="ic ph"><Icon name="extension" /></span>
            <div class="meta">
              <strong>{{ r.display_name || r.displayName || r.name || r.id }} <small>{{ r.version }}</small></strong>
              <span class="desc">{{ r.description }}</span>
            </div>
            <button v-if="!byId.get(r.id)" class="btn tonal sm" :disabled="busy === r.id" @click="install(r)"><Icon name="download" />{{ busy === r.id ? "Memasang…" : "Pasang" }}</button>
            <button v-else-if="newer(r.version, byId.get(r.id)!.version)" class="btn filled sm" :disabled="busy === r.id" @click="install(r)"><Icon name="sync" />{{ busy === r.id ? "Memperbarui…" : "Perbarui" }}</button>
            <span v-else class="ok"><Icon name="done" :size="20" />Terpasang</span>
          </div>
        </li>
      </ul>
    </section>
  </div>
</template>

<style scoped>
.exts { display: flex; flex-direction: column; gap: 16px; }
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
@media (max-width: 600px) { .form { padding-left: 16px; } }
</style>
