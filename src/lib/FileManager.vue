<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { confirm } from "./ConfirmDialog.vue";
import Icon from "./Icon.vue";

// File manager for a library folder (`kind` "local") or the configured bucket ("s3"), backed by the
// `<kind>_list/_mkdir/_upload/_delete` commands. `root` is "" or ends in "/"; navigation never goes above it.
// Changes to songs come back as the new library via `library`.
const props = defineProps<{ kind: "local" | "s3"; root: string; label: string }>();
const emit = defineEmits<{ library: [tracks: unknown[]] }>();

type Entry = { key: string; name: string; dir: boolean; size: number; modified: string };
const AUDIO = ["mp3", "flac", "m4a", "aac", "ogg", "opus", "wav"];

const cwd = ref(props.root);
const entries = ref<Entry[]>([]);
const loading = ref(false);
const busy = ref(""); // what a running upload/delete is doing
const error = ref("");
const selected = ref(new Set<string>());
const newName = ref<string | null>(null);
const newInput = ref<HTMLInputElement>();
const dropping = ref(false);
// feedback while the backend works: rows that are on their way in or out, and upload progress
// per file: 0 waiting, 1 sending, 2 done (several are sent at once)
type Prog = { states: number[]; name: string; done: number; total: number };
const prog = ref<Prog | null>(null);
const incoming = ref<{ name: string; dir: boolean }[]>([]);
const removing = ref(new Set<string>());
// transfer speed over the last few seconds of progress events (S3 reports every 5 MB part)
const samples: { t: number; done: number }[] = [];
const speed = ref(0); // bytes per second
function sample(done: number) {
  const t = performance.now();
  samples.push({ t, done });
  while (samples.length > 2 && t - samples[0].t > 6000) samples.shift();
  const first = samples[0];
  if (t - first.t > 300) speed.value = (done - first.done) / ((t - first.t) / 1000);
}
const eta = computed(() => {
  if (!prog.value?.total || speed.value <= 0) return "";
  const s = Math.ceil((prog.value.total - prog.value.done) / speed.value);
  return s < 60 ? `${s} dtk lagi` : `${Math.floor(s / 60)} mnt ${s % 60} dtk lagi`;
});
const pct = computed(() => (prog.value?.total ? Math.min(100, (prog.value.done / prog.value.total) * 100) : 0));
// files being uploaded show their place in the queue; a new folder shows up straight away
const rowState = (i: number) => (["queued", "active", "done"] as const)[prog.value ? prog.value.states[i] ?? 0 : 1];
const finished = computed(() => prog.value?.states.filter((s) => s === 2).length ?? 0);
const shownEntries = computed(() => {
  const coming = new Set(incoming.value.map((p) => p.name));
  return entries.value.filter((e) => !coming.has(e.name));
});

const crumbs = computed(() => {
  const parts = cwd.value.slice(props.root.length).split("/").filter(Boolean);
  return parts.map((name, i) => ({ name, key: props.root + parts.slice(0, i + 1).join("/") + "/" }));
});
const allSelected = computed(() => entries.value.length > 0 && selected.value.size === entries.value.length);

// Listings are cached per folder and requests shared, so opening a folder shows what is known at once
// (then refreshes it), and hovering a folder fetches it ahead of the click.
const cache = new Map<string, Entry[]>();
const inflight = new Map<string, Promise<Entry[]>>();
function fetchList(prefix: string) {
  let p = inflight.get(prefix);
  if (!p) {
    p = invoke<Entry[]>(`${props.kind}_list`, { prefix })
      .then((list) => (cache.set(prefix, list), list))
      .finally(() => inflight.delete(prefix));
    inflight.set(prefix, p);
  }
  return p;
}
const prefetch = (prefix: string) => cache.has(prefix) || fetchList(prefix).catch(() => {});

async function load(prefix = cwd.value) {
  const moved = prefix !== cwd.value;
  cwd.value = prefix;
  error.value = "";
  if (moved) {
    selected.value = new Set();
    entries.value = cache.get(prefix) ?? [];
  }
  loading.value = true;
  try {
    // a listing already on its way is reused only when navigating; a refresh asks again
    if (!moved) inflight.delete(prefix);
    const list = await fetchList(prefix);
    if (cwd.value === prefix) entries.value = list;
  } catch (e) {
    if (cwd.value === prefix) error.value = String(e);
  } finally {
    if (cwd.value === prefix) loading.value = false;
  }
}

async function run(label: string, cmd: string, args: Record<string, unknown>) {
  busy.value = label;
  error.value = "";
  try {
    emit("library", await invoke<unknown[]>(cmd, args));
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
    await load();
  }
}

async function upload(files: string[]) {
  if (!files.length) return;
  incoming.value = files.map((f) => ({ name: f.split(/[\\/]/).pop()!, dir: false }));
  prog.value = { states: files.map(() => 0), name: incoming.value[0].name, done: 0, total: 0 };
  samples.length = 0;
  speed.value = 0;
  await run(props.kind === "s3" ? "Mengunggah…" : "Menyalin…", `${props.kind}_upload`, { prefix: cwd.value, files });
  incoming.value = [];
  prog.value = null;
}
async function pickUpload() {
  const files = await open({ multiple: true });
  if (files) await upload(files);
}

async function remove(keys: string[]) {
  const what = keys.length === 1 ? `"${entries.value.find((e) => e.key === keys[0])?.name}"` : `${keys.length} item`;
  const dirs = keys.some((k) => k.endsWith("/"));
  const msg = props.kind === "s3"
    ? `Item akan dihapus permanen dari bucket.${dirs ? " Semua isi folder ikut terhapus." : ""} Tindakan ini tidak bisa dibatalkan.`
    : `Item masih bisa dipulihkan dari tempat sampah sistem.${dirs ? " Semua isi folder ikut dipindahkan." : ""}`;
  const ok = await confirm({
    title: props.kind === "s3" ? `Hapus ${what}?` : `Pindahkan ${what} ke tempat sampah?`,
    message: msg,
    confirm: props.kind === "s3" ? "Hapus" : "Pindahkan",
    icon: "trash",
    danger: true,
  });
  if (!ok) return;
  removing.value = new Set(keys);
  await run(keys.length > 1 ? `Menghapus ${keys.length} item…` : "Menghapus…", `${props.kind}_delete`, { keys });
  removing.value = new Set();
}

async function startFolder() {
  newName.value = "";
  await nextTick();
  newInput.value?.focus();
}
async function createFolder() {
  const name = newName.value?.trim().replace(/\//g, "");
  newName.value = null;
  if (!name) return;
  busy.value = "Membuat folder…";
  incoming.value = [{ name, dir: true }];
  try {
    await invoke(`${props.kind}_mkdir`, { prefix: `${cwd.value}${name}/` });
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
    await load();
    incoming.value = [];
  }
}

function toggle(key: string) {
  const s = new Set(selected.value);
  s.has(key) ? s.delete(key) : s.add(key);
  selected.value = s;
}
const toggleAll = () => (selected.value = allSelected.value ? new Set() : new Set(entries.value.map((e) => e.key)));

const isAudio = (name: string) => AUDIO.includes(name.split(".").pop()?.toLowerCase() ?? "");
function size(n: number) {
  if (n < 1024) return `${n} B`;
  const u = ["KB", "MB", "GB", "TB"];
  let i = -1;
  do {
    n /= 1024;
    i++;
  } while (n >= 1024 && i < u.length - 1);
  return `${n.toFixed(n < 10 ? 1 : 0)} ${u[i]}`;
}
const date = (s: string) => (s ? new Date(/^\d+$/.test(s) ? +s : s).toLocaleDateString("id-ID", { day: "numeric", month: "short", year: "numeric" }) : "");

watch(() => props.root, (r) => load(r));

defineExpose({ reload: () => load() });

const subs: Promise<UnlistenFn>[] = [];
onMounted(() => {
  load();
  subs.push(
    listen<Prog>(`${props.kind}-upload`, ({ payload }) => {
      if (!prog.value) return;
      prog.value = payload;
      sample(payload.done);
    }),
    // files may change outside the app (restored from the trash, copied in a file manager)
    getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused && !busy.value) load();
    }),
    // files dropped onto the window go into the open folder
    getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === "over" || payload.type === "enter") dropping.value = true;
      else if (payload.type === "leave") dropping.value = false;
      else if (payload.type === "drop") {
        dropping.value = false;
        if (!busy.value) upload(payload.paths);
      }
    }),
  );
});
onUnmounted(() => subs.forEach((s) => s.then((f) => f())));
</script>

<template>
  <section class="files" :class="{ dropping }">
    <div class="bar">
      <nav class="crumbs" aria-label="Lokasi">
        <button class="crumb" :class="{ cur: !crumbs.length }" @click="load(root)" :title="root">
          <Icon name="home" :size="20" />{{ label }}
        </button>
        <template v-for="(c, i) in crumbs" :key="c.key">
          <Icon name="chevron" :size="18" class="sep" />
          <button class="crumb" :class="{ cur: i === crumbs.length - 1 }" @click="load(c.key)">{{ c.name }}</button>
        </template>
        <span v-if="loading && entries.length" class="bar-anim" aria-label="Memuat"></span>
      </nav>
      <div class="tools">
        <button v-if="selected.size" class="btn danger" :disabled="!!busy" @click="remove([...selected])">
          <Icon name="trash" :size="20" />Hapus {{ selected.size }}
        </button>
        <button class="icon-btn" title="Muat ulang" :disabled="loading || !!busy" @click="load()"><Icon name="refresh" /></button>
        <button class="btn tonal" :disabled="!!busy" @click="startFolder"><Icon name="newFolder" :size="20" />Folder baru</button>
        <button class="btn filled" :disabled="!!busy" @click="pickUpload"><Icon name="upload" :size="20" />{{ kind === "s3" ? "Unggah" : "Tambah file" }}</button>
      </div>
    </div>

    <div v-if="prog" class="progress" role="progressbar" :aria-valuenow="Math.round(pct)" aria-valuemin="0" aria-valuemax="100">
      <div class="p-text">
        <strong>{{ kind === "s3" ? "Mengunggah" : "Menyalin" }} · {{ finished }} dari {{ prog.states.length }} selesai</strong>
        <span class="p-name">{{ prog.name }}</span>
        <span class="p-num">{{ prog.total ? `${size(prog.done)} / ${size(prog.total)} · ${Math.round(pct)}%` : "Menyiapkan…" }}</span>
      </div>
      <div v-if="prog.total" class="p-speed">
        <Icon name="upload" :size="16" />
        <span>{{ speed > 0 ? `${size(speed)}/s` : "Mengukur kecepatan…" }}</span>
        <span v-if="eta" class="p-eta">{{ eta }}</span>
      </div>
      <div class="track"><div class="fill" :class="{ indet: !prog.total }" :style="{ width: `${prog.total ? pct : 30}%` }"></div></div>
    </div>
    <p v-else-if="busy" class="note" role="status"><span class="bar-anim"></span>{{ busy }}</p>
    <p v-if="error" class="note err" role="alert">{{ error }}</p>

    <div class="table" role="table" aria-label="Isi folder">
      <div class="row head" role="row">
        <input type="checkbox" :checked="allSelected" :indeterminate="selected.size > 0 && !allSelected" @change="toggleAll" aria-label="Pilih semua" />
        <span>Nama</span><span class="num">Ukuran</span><span class="num">Diubah</span><span></span>
      </div>

      <form v-if="newName !== null" class="row" @submit.prevent="createFolder">
        <span></span>
        <span class="name"><Icon name="folder" class="ic dir" />
          <input ref="newInput" v-model="newName" class="rename" placeholder="Nama folder" @blur="createFolder" @keydown.esc="newName = null" />
        </span>
      </form>

      <TransitionGroup name="rows">
      <div v-for="(p, i) in incoming" :key="`+${p.name}`" class="row pending" role="row">
        <span class="state" :class="rowState(i)">
          <span v-if="rowState(i) === 'active'" class="bar-anim"></span>
          <Icon v-else-if="rowState(i) === 'done'" name="downloadDone" :size="20" />
        </span>
        <span class="name"><Icon :name="p.dir ? 'folder' : isAudio(p.name) ? 'note' : 'file'" class="ic" :class="{ dir: p.dir }" /><span>{{ p.name }}</span></span>
        <span class="num">{{ p.dir ? "Membuat…" : rowState(i) === "queued" ? "Menunggu" : rowState(i) === "done" ? "Selesai" : "Mengirim…" }}</span>
        <span class="num"></span><span></span>
      </div>
      <div v-for="e in shownEntries" :key="e.key" class="row" :class="{ sel: selected.has(e.key), gone: removing.has(e.key) }" role="row"
        @dblclick="e.dir && cwd !== e.key && load(e.key)">
        <input type="checkbox" :checked="selected.has(e.key)" @change="toggle(e.key)" :aria-label="`Pilih ${e.name}`" />
        <button v-if="e.dir" class="name link" @click="load(e.key)" @pointerenter="prefetch(e.key)" @focus="prefetch(e.key)"><Icon name="folder" class="ic dir" /><span>{{ e.name }}</span></button>
        <span v-else class="name"><Icon :name="isAudio(e.name) ? 'note' : 'file'" class="ic" :class="{ audio: isAudio(e.name) }" /><span>{{ e.name }}</span></span>
        <span class="num">{{ e.dir ? "—" : size(e.size) }}</span>
        <span class="num">{{ date(e.modified) }}</span>
        <button class="icon-btn small" :disabled="!!busy" :title="`Hapus ${e.name}`" @click="remove([e.key])"><Icon name="trash" :size="20" /></button>
      </div>
      </TransitionGroup>

      <div v-if="!loading && !entries.length && !incoming.length && newName === null" class="blank">
        <Icon name="upload" :size="40" />
        <p>Folder ini kosong. {{ kind === "s3" ? "Unggah" : "Tambahkan" }} lagu, atau seret file ke jendela ini.</p>
      </div>
      <template v-if="loading && !entries.length">
        <div v-for="n in 6" :key="`sk${n}`" class="row skel" aria-hidden="true">
          <span></span><span class="name"><i class="sk-ic"></i><i class="sk-bar" :style="{ width: `${40 + ((n * 37) % 45)}%` }"></i></span>
          <span class="num"><i class="sk-bar short"></i></span><span class="num"><i class="sk-bar short"></i></span><span></span>
        </div>
      </template>
    </div>
  </section>
</template>

<style scoped>
.files {
  position: relative; display: flex; flex-direction: column; gap: 12px; padding: 20px; border-radius: 28px;
  background: var(--surface-container-low); outline: 2px dashed transparent; outline-offset: -2px; transition: outline-color 0.2s;
}
.files.dropping { outline-color: var(--primary); background: color-mix(in srgb, var(--primary) 6%, var(--surface-container-low)); }

.bar { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 12px; }
.crumbs { display: flex; align-items: center; flex-wrap: wrap; gap: 2px; min-width: 0; }
.crumb {
  display: inline-flex; align-items: center; gap: 6px; height: 36px; padding: 0 12px; border-radius: 18px;
  font-size: 15px; font-weight: 600; color: var(--on-surface-variant); transition: background 0.2s;
}
.crumb:hover { background: var(--surface-container-highest); }
.crumb.cur { color: var(--on-surface); }
.sep { color: var(--on-surface-variant); opacity: 0.6; }
.tools { display: flex; align-items: center; gap: 8px; }

.btn {
  display: inline-flex; align-items: center; gap: 8px; height: 40px; padding: 0 20px 0 16px; border-radius: 20px;
  font-size: 14px; font-weight: 600; white-space: nowrap; transition: border-radius 0.3s var(--spring), background 0.2s;
}
.btn:active { border-radius: 10px; }
.btn:disabled, .icon-btn:disabled { opacity: 0.5; cursor: default; }
.filled { background: var(--primary); color: var(--on-primary); }
.tonal { background: var(--secondary-container); color: var(--on-secondary-container); }
.danger { background: #8c1d18; color: #ffdad6; }
.icon-btn {
  width: 40px; height: 40px; border-radius: 20px; display: inline-grid; place-items: center; flex: none;
  color: var(--on-surface-variant); transition: background 0.2s, border-radius 0.3s var(--spring);
}
.icon-btn:hover:not(:disabled) { background: var(--surface-container-highest); }
.icon-btn.small { width: 36px; height: 36px; }

.note { display: flex; align-items: center; gap: 12px; margin: 0; padding: 10px 16px; border-radius: 12px; background: var(--secondary-container); color: var(--on-secondary-container); font-size: 14px; }
.note.err { background: #8c1d18; color: #ffdad6; }
.bar-anim { width: 16px; height: 16px; border-radius: 50%; border: 2px solid currentColor; border-right-color: transparent; animation: spin 0.8s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }

.progress { display: flex; flex-direction: column; gap: 10px; padding: 14px 16px; border-radius: 16px; background: var(--secondary-container); color: var(--on-secondary-container); }
.p-text { display: flex; align-items: baseline; gap: 12px; min-width: 0; font-size: 14px; }
.p-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; opacity: 0.8; }
.p-num { flex: none; font-variant-numeric: tabular-nums; opacity: 0.8; }
.p-speed { display: flex; align-items: center; gap: 6px; font-size: 13px; font-weight: 600; font-variant-numeric: tabular-nums; order: 1; }
.p-eta { margin-left: auto; font-weight: 500; opacity: 0.8; }
.track { height: 4px; border-radius: 2px; overflow: hidden; background: color-mix(in srgb, currentColor 20%, transparent); }
.fill { height: 100%; border-radius: 2px; background: var(--primary); transition: width 0.4s var(--emph); }
.fill.indet { animation: indet 1.2s var(--emph) infinite; }
@keyframes indet { from { transform: translateX(-100%); } to { transform: translateX(340%); } }

.table { display: flex; flex-direction: column; }
.row.skel:hover { background: none; }
.sk-ic, .sk-bar { display: block; border-radius: 6px; background: var(--surface-container-highest); animation: shimmer 1.2s ease-in-out infinite alternate; }
.sk-ic { width: 24px; height: 24px; border-radius: 8px; flex: none; }
.sk-bar { height: 12px; }
.sk-bar.short { width: 56px; margin-left: auto; }
@keyframes shimmer { from { opacity: 0.45; } to { opacity: 1; } }
.crumbs .bar-anim { width: 14px; height: 14px; margin-left: 8px; color: var(--primary); }
.row.pending { color: var(--on-surface-variant); }
.row.pending .name { opacity: 0.75; }
.row.gone { opacity: 0.4; pointer-events: none; transition: opacity 0.2s; }
.state { width: 40px; display: grid; place-items: center; color: var(--primary); }
.state .bar-anim { color: var(--primary); }
.rows-enter-active { transition: opacity 0.25s var(--emph), transform 0.35s var(--spring); }
.rows-enter-from { opacity: 0; transform: translateY(-8px); }
.row {
  display: grid; grid-template-columns: 40px minmax(0, 1fr) 96px 120px 40px; align-items: center; gap: 8px;
  min-height: 52px; padding: 0 4px 0 8px; border-radius: 14px; transition: background 0.15s;
}
.row:not(.head):hover { background: var(--surface-container-high); }
.row.sel { background: var(--secondary-container); color: var(--on-secondary-container); }
.row.head { min-height: 40px; font-size: 13px; font-weight: 600; color: var(--on-surface-variant); border-bottom: 1px solid var(--outline-variant); border-radius: 0; margin-bottom: 4px; }
.row input[type="checkbox"] { width: 18px; height: 18px; justify-self: center; accent-color: var(--primary); cursor: pointer; }
.row .icon-btn { opacity: 0; }
.row:hover .icon-btn, .row .icon-btn:focus-visible { opacity: 1; }
.name { display: flex; align-items: center; gap: 12px; min-width: 0; font-size: 15px; text-align: left; }
.name > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.link { font-weight: 600; }
.link:hover > span { text-decoration: underline; }
.ic { color: var(--on-surface-variant); }
.ic.dir { color: var(--primary); }
.ic.audio { color: var(--tertiary, var(--primary)); }
.num { text-align: right; font-size: 14px; color: var(--on-surface-variant); font-variant-numeric: tabular-nums; white-space: nowrap; }
.row.sel .num, .row.sel .ic { color: inherit; }
.rename {
  flex: 1; min-width: 0; height: 36px; padding: 0 12px; border: 0; border-radius: 8px; outline: none;
  background: var(--surface-container-highest); color: var(--on-surface); font-size: 15px; box-shadow: inset 0 -2px 0 var(--primary);
}
.blank { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 48px 16px; color: var(--on-surface-variant); text-align: center; }
.blank p { margin: 0; }

@media (max-width: 760px) {
  .row { grid-template-columns: 40px minmax(0, 1fr) 80px 40px; }
  .row > :nth-child(4) { display: none; }
}
/* touch screens: no hover, so a row's menu button is always there */
@media (hover: none) { .row .icon-btn { opacity: 1; } }
/* phones: the tools get their own full-width line under the path */
@media (max-width: 600px) {
  .files { padding: 16px 12px; border-radius: 24px; }
  .tools { flex-wrap: wrap; width: 100%; }
  .tools .btn { flex: 1 1 0; min-width: 0; justify-content: center; gap: 6px; padding: 0 12px 0 10px; }
  .row { grid-template-columns: 32px minmax(0, 1fr) 64px 40px; gap: 4px; padding: 0 0 0 4px; }
}
</style>
