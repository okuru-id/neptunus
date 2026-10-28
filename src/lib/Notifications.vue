<script setup lang="ts">
import { onUnmounted, ref } from "vue";
import Icon from "./Icon.vue";

// Bell in the title bar: running and queued downloads plus a history of notices (failures, finished
// downloads, a new release). The badge counts running jobs and unread notices.
export type Job = { key: string; title: string; pct: number | null; queued?: boolean; detail?: string; cancel?: () => void };
export type Note = { id: number; kind: "error" | "done" | "update"; text: string; time: number; url?: string };
const props = defineProps<{ jobs: Job[]; notes: Note[]; unread: number }>();
const emit = defineEmits<{ read: []; clear: []; cancelAll: []; open: [url: string] }>();

const open = ref(false);
const root = ref<HTMLElement>();
const outside = (e: PointerEvent) => !root.value?.contains(e.target as Node) && close();
function close() {
  open.value = false;
  window.removeEventListener("pointerdown", outside);
}
function toggle() {
  if (open.value) return close();
  open.value = true;
  emit("read");
  window.addEventListener("pointerdown", outside);
}
onUnmounted(close);
// opened and closed by the app too: while a mix is made
defineExpose({ show: () => open.value || toggle(), close });

const ICON = { error: "error", done: "downloadDone", update: "update" } as const;
const time = (t: number) => new Date(t).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
const badge = () => props.jobs.length + props.unread;
</script>

<template>
  <div ref="root" class="notif">
    <button class="bell" :class="{ on: open }" title="Notifikasi dan unduhan" aria-label="Notifikasi dan unduhan" :aria-expanded="open" @click="toggle">
      <Icon :name="jobs.length ? 'bellActive' : 'bell'" :size="18" />
      <span v-if="badge()" class="badge">{{ badge() > 99 ? "99+" : badge() }}</span>
    </button>
    <Transition name="drop">
      <div v-if="open" class="panel" role="dialog" aria-label="Notifikasi">
        <section v-if="jobs.length">
          <header>
            <h3>Sedang berjalan</h3>
            <button v-if="jobs.some((j) => j.cancel)" class="txt" @click="emit('cancelAll')">Batalkan semua</button>
          </header>
          <ul>
            <li v-for="j in jobs" :key="j.key" class="job">
              <span class="ic" :class="{ busy: !j.queued }"><Icon :name="j.queued ? 'download' : 'sync'" :size="18" /></span>
              <div class="body">
                <strong>{{ j.title }}</strong>
                <span v-if="j.queued">Dalam antrean</span>
                <span v-else-if="j.pct != null" class="bar"><i :style="{ width: `${j.pct}%` }"></i></span>
                <span v-if="!j.queued && j.detail" class="detail">{{ j.detail }}</span>
              </div>
              <span v-if="!j.queued && j.pct != null" class="pct">{{ Math.round(j.pct) }}%</span>
              <button v-if="j.cancel" class="x" title="Batalkan" :aria-label="`Batalkan ${j.title}`" @click="j.cancel()"><Icon name="close" :size="18" /></button>
            </li>
          </ul>
        </section>
        <section>
          <header>
            <h3>Notifikasi</h3>
            <button v-if="notes.length" class="x" title="Hapus semua notifikasi" aria-label="Hapus semua notifikasi" @click="emit('clear')"><Icon name="clearAll" :size="20" /></button>
          </header>
          <ul v-if="notes.length">
            <li v-for="n in notes" :key="n.id" class="note" :class="n.kind">
              <span class="ic"><Icon :name="ICON[n.kind]" :size="18" /></span>
              <div class="body">
                <span class="text">{{ n.text }}</span>
                <span class="time">{{ time(n.time) }}</span>
              </div>
              <button v-if="n.url" class="txt" @click="emit('open', n.url)">Lihat</button>
            </li>
          </ul>
          <p v-else class="empty">Belum ada notifikasi.</p>
        </section>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.notif { position: relative; }
.bell {
  position: relative; width: 44px; height: 30px; border-radius: 15px; display: grid; place-items: center; color: var(--on-surface-variant);
  transition: background 0.15s, color 0.15s, border-radius 0.3s var(--spring);
}
.bell.on { background: var(--surface-container-highest); color: var(--on-surface); }
@media (hover: hover) { .bell:hover { background: var(--surface-container-highest); color: var(--on-surface); } }
.bell:active { border-radius: 8px; }
.badge {
  position: absolute; top: 1px; right: 6px; min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px; box-sizing: border-box;
  display: grid; place-items: center; font-size: 10px; font-weight: 700; line-height: 1; font-variant-numeric: tabular-nums;
  background: var(--error, #b3261e); color: var(--on-error, #fff);
}
.panel {
  position: absolute; top: calc(100% + 8px); right: 0; z-index: 40; width: min(380px, calc(100vw - 32px)); max-height: min(70vh, 560px); overflow-y: auto;
  padding: 8px 0; border-radius: 24px; background: var(--surface-container-high); color: var(--on-surface); box-shadow: 0 8px 28px rgb(0 0 0 / 0.32);
}
section + section { border-top: 1px solid var(--outline-variant); margin-top: 4px; padding-top: 4px; }
header { display: flex; align-items: center; justify-content: space-between; padding: 6px 12px 4px 20px; }
h3 { margin: 0; font-size: 13px; font-weight: 700; color: var(--on-surface-variant); }
.txt { height: 32px; padding: 0 12px; border-radius: 16px; font-size: 13px; font-weight: 600; color: var(--primary); flex: none; }
.txt:hover { background: color-mix(in srgb, var(--primary) 8%, transparent); }
ul { list-style: none; margin: 0; padding: 0; }
li { display: flex; align-items: center; gap: 12px; padding: 8px 12px 8px 16px; }
.ic { width: 36px; height: 36px; flex: none; display: grid; place-items: center; border-radius: 12px; background: var(--surface-container-highest); color: var(--on-surface-variant); }
.ic.busy { background: var(--primary-container); color: var(--on-primary-container); }
.ic.busy :deep(svg) { animation: spin 1.6s linear infinite; }
.note.error .ic { background: var(--error-container, #f9dedc); color: var(--on-error-container, #410e0b); }
.note.done .ic, .note.update .ic { background: var(--primary-container); color: var(--on-primary-container); }
.body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--on-surface-variant); }
.detail { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.body strong { font-size: 14px; font-weight: 600; color: var(--on-surface); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.text { font-size: 13px; color: var(--on-surface); line-height: 1.4; overflow-wrap: anywhere; }
.time { font-size: 11px; font-variant-numeric: tabular-nums; }
.bar { height: 4px; border-radius: 2px; background: var(--secondary-container); overflow: hidden; }
.bar i { display: block; height: 100%; border-radius: 2px; background: var(--primary); transition: width 0.3s; }
.pct { font-size: 12px; font-weight: 700; color: var(--primary); font-variant-numeric: tabular-nums; flex: none; }
.x { width: 32px; height: 32px; flex: none; border-radius: 16px; display: grid; place-items: center; color: var(--on-surface-variant); }
.x:hover { background: var(--surface-container-highest); }
.empty { margin: 8px 20px 12px; font-size: 13px; color: var(--on-surface-variant); }
.drop-enter-active, .drop-leave-active { transition: opacity 0.15s, transform 0.25s var(--emph); transform-origin: top right; }
.drop-enter-from, .drop-leave-to { opacity: 0; transform: translateY(-6px) scale(0.97); }
/* phones: the bell isn't at the window edge, so the panel spans the screen under the app bar */
@media (max-width: 600px) {
  .panel { position: fixed; top: calc(var(--titlebar) + 4px); left: 8px; right: 8px; width: auto; }
}
@keyframes spin { to { transform: rotate(360deg); } }
</style>
