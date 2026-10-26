<script lang="ts">
import { reactive } from "vue";
import type { IconName } from "./icons";

type Opts = { title: string; message: string; confirm?: string; cancel?: string; icon?: IconName; danger?: boolean };
const state = reactive<{ opts: Opts | null; resolve: (ok: boolean | null) => void }>({ opts: null, resolve: () => {} });

/** A choice between two answers: true for `confirm`, false for `cancel`, null when dismissed (Esc,
 *  a click beside it). */
export function choose(opts: Opts): Promise<boolean | null> {
  state.resolve(null); // a dialog already open counts as dismissed
  return new Promise((resolve) => {
    state.opts = opts;
    state.resolve = resolve;
  });
}

/** In-app replacement for the system `ask()` dialog; resolves true when confirmed. */
export const confirm = (opts: Opts) => choose(opts).then((ok) => ok === true);
</script>

<script setup lang="ts">
import { nextTick, ref, watch } from "vue";
import Icon from "./Icon.vue";

// M3 basic dialog. Mount once (in App); open it with `confirm()`.
const cancelBtn = ref<HTMLButtonElement>();
function close(ok: boolean | null) {
  state.opts = null;
  state.resolve(ok);
  state.resolve = () => {};
}
// destructive actions start on Cancel, so a stray Enter doesn't delete anything
watch(() => state.opts, async (o) => o && (await nextTick(), cancelBtn.value?.focus()));
</script>

<template>
  <Transition name="dialog">
    <div v-if="state.opts" class="scrim" @pointerdown.self="close(null)" @keydown.esc="close(null)">
      <div class="dialog" :class="{ danger: state.opts.danger }" role="alertdialog" aria-modal="true" aria-labelledby="dlg-title" aria-describedby="dlg-msg">
        <span v-if="state.opts.icon" class="badge" :style="{ clipPath: 'url(#shape-cookie9)' }"><Icon :name="state.opts.icon" :size="32" /></span>
        <h2 id="dlg-title">{{ state.opts.title }}</h2>
        <p id="dlg-msg">{{ state.opts.message }}</p>
        <div class="actions">
          <button ref="cancelBtn" class="btn tonal" @click="close(false)">{{ state.opts.cancel ?? "Batal" }}</button>
          <button class="btn filled" @click="close(true)">{{ state.opts.confirm ?? "OK" }}</button>
        </div>
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
  --accent: var(--primary); --on-accent: var(--on-primary);
  --accent-container: var(--primary-container); --on-accent-container: var(--on-primary-container);
  position: relative; overflow: hidden;
  display: flex; flex-direction: column; align-items: center; text-align: center;
  width: min(400px, 100%); padding: 32px 28px 24px; border-radius: 32px;
  background:
    radial-gradient(120% 70% at 50% 0%, color-mix(in srgb, var(--accent) 14%, transparent), transparent 70%),
    var(--surface-container-high);
  color: var(--on-surface);
  box-shadow: 0 24px 64px rgb(0 0 0 / 0.35), inset 0 0 0 1px color-mix(in srgb, var(--on-surface) 6%, transparent);
}
.dialog.danger { --accent: var(--error); --on-accent: var(--on-error); --accent-container: var(--error-container); --on-accent-container: var(--on-error-container); }
.badge {
  width: 72px; height: 72px; display: grid; place-items: center; margin-bottom: 20px;
  background: var(--accent-container); color: var(--on-accent-container);
  animation: pop 0.5s var(--spring);
}
@keyframes pop { from { transform: scale(0.4) rotate(-40deg); } }
h2 {
  margin: 0 0 8px; font-size: 24px; line-height: 1.25; font-weight: 750; letter-spacing: -0.01em;
  font-variation-settings: "wdth" 112; overflow-wrap: anywhere;
}
p { margin: 0; font-size: 15px; line-height: 1.5; color: var(--on-surface-variant); max-width: 34ch; }
.actions { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; width: 100%; margin-top: 28px; }
.btn {
  height: 48px; padding: 0 20px; border-radius: 24px; font-size: 15px; font-weight: 600;
  transition: border-radius 0.3s var(--spring), filter 0.2s, box-shadow 0.2s;
}
.btn:hover { filter: brightness(1.08); }
.btn:active { border-radius: 12px; }
.btn:focus-visible { outline: none; box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 45%, transparent); }
.tonal { background: var(--surface-container-highest); color: var(--on-surface); }
.filled { background: var(--accent); color: var(--on-accent); }

.dialog-enter-active, .dialog-leave-active { transition: opacity 0.25s var(--emph); }
.dialog-enter-active .dialog { transition: transform 0.45s var(--spring), opacity 0.25s; }
.dialog-leave-active .dialog { transition: transform 0.2s var(--emph), opacity 0.2s; }
.dialog-enter-from, .dialog-leave-to { opacity: 0; }
.dialog-enter-from .dialog { transform: translateY(24px) scale(0.92); opacity: 0; }
.dialog-leave-to .dialog { transform: scale(0.96); opacity: 0; }
</style>
