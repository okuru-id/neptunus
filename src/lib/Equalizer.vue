<script lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { ref } from "vue";

// Settings › Equalizer: 10 peaking bands (gain ±12 dB, any center frequency). The state lives at module
// level so the saved curve is sent to the player at startup, before the tab is ever opened.
// low shelf, 8 octave-spaced peaks, high shelf: lets smooth curves (presets) fit within ~1 dB
const DEF_FREQS = [63, 100, 200, 400, 800, 1600, 3150, 6300, 10000, 12500];
const MAX_DB = 12;
const load = (k: string, def: number[]) => {
  try {
    const v = JSON.parse(localStorage.getItem(k) ?? "");
    if (Array.isArray(v) && v.length === 10 && v.every(Number.isFinite)) return v as number[];
  } catch {}
  return def.slice();
};
const gains = ref(load("eq", DEF_FREQS.map(() => 0)));
const freqs = ref(load("eqFreq", DEF_FREQS));

function apply() {
  gains.value = gains.value.map((g) => Math.min(MAX_DB, Math.max(-MAX_DB, Math.round(g * 2) / 2 || 0)));
  freqs.value = freqs.value.map((f) => Math.min(20000, Math.max(20, Math.round(f) || 20)));
  localStorage.setItem("eq", JSON.stringify(gains.value));
  localStorage.setItem("eqFreq", JSON.stringify(freqs.value));
  invoke("set_eq", { gains: gains.value.map((g) => Math.round(g * 10)), freqs: freqs.value }).catch(() => {});
}
apply();

// Same RBJ biquads as the player (`biquad` in player.rs), so the drawn curve is what you hear:
// band 1 is a low shelf, band 10 a high shelf, the rest peaking filters (Q 1.4).
const FS = 48000;
function coef(band: number, f: number, db: number) {
  const a = 10 ** (db / 40), w = (2 * Math.PI * f) / FS, c = Math.cos(w), s = Math.sin(w);
  let k: number[];
  if (band === 0 || band === 9) {
    const q = (2 * Math.sqrt(a) * s) / Math.SQRT2, g = band === 0 ? 1 : -1;
    k = [a * (a + 1 - g * (a - 1) * c + q), g * 2 * a * (a - 1 - g * (a + 1) * c), a * (a + 1 - g * (a - 1) * c - q),
      a + 1 + g * (a - 1) * c + q, -g * 2 * (a - 1 + g * (a + 1) * c), a + 1 + g * (a - 1) * c - q];
  } else {
    const al = s / 2.8;
    k = [1 + al * a, -2 * c, 1 - al * a, 1 + al / a, -2 * c, 1 - al / a];
  }
  return [k[0] / k[3], k[1] / k[3], k[2] / k[3], k[4] / k[3], k[5] / k[3]];
}
function magDb(k: number[], f: number) {
  const w = (2 * Math.PI * f) / FS, c1 = Math.cos(w), s1 = Math.sin(w), c2 = Math.cos(2 * w), s2 = Math.sin(2 * w);
  const num = (k[0] + k[1] * c1 + k[2] * c2) ** 2 + (k[1] * s1 + k[2] * s2) ** 2;
  const den = (1 + k[3] * c1 + k[4] * c2) ** 2 + (k[3] * s1 + k[4] * s2) ** 2;
  return 10 * Math.log10(num / den);
}
const response = (g: number[], fr: number[], f: number) => g.reduce((s, db, i) => s + (db ? magDb(coef(i, fr[i], db), f) : 0), 0);

// Harman over-ear target 2018 (Olive et al.), from AutoEq's "Harman over-ear 2018.csv", normalised to 0 dB at
// 1 kHz and sampled here. It is the response at the eardrum, so it includes the ear-gain rise around 3 kHz.
// Above 16 kHz the data only reflects the measurement rig, so the line stops there.
const HARMAN: [number, number][] = [
  [20, 3.9], [30, 4], [45, 3.5], [60, 2.9], [80, 2], [100, 1], [150, -0.9], [200, -2], [300, -1.7], [400, -1.1],
  [500, -0.8], [700, -0.3], [1000, 0], [1500, 2.1], [2000, 5.3], [2500, 7.4], [3000, 8.3], [4000, 8.2], [5000, 6.4],
  [6000, 5.1], [8000, 2.4], [10000, -0.7], [12000, -4.2], [15000, -8], [16000, -9.2],
];
const HARMAN_MAX = 16000;

// A target curve as [Hz, dB] points, read with log-frequency interpolation (flat beyond the ends).
type Curve = [number, number][];
function interp(c: Curve, f: number) {
  const i = c.findIndex(([x]) => x >= f);
  if (i <= 0) return c[i < 0 ? c.length - 1 : 0][1];
  const [[x0, y0], [x1, y1]] = [c[i - 1], c[i]];
  return y0 + ((y1 - y0) * Math.log(f / x0)) / Math.log(x1 / x0);
}
const harman = (f: number) => interp(HARMAN, f);

// Least-squares fit of band gains to a curve over 20 Hz..fmax: each band's dB response is close to linear in
// its gain, so its 1 dB shape is the basis. Returns gains rounded like `apply` does.
function fit(c: Curve, fr: number[], fmax = 20000) {
  const xs = Array.from({ length: 121 }, (_, k) => 20 * (fmax / 20) ** (k / 120));
  const basis = fr.map((f, i) => xs.map((x) => magDb(coef(i, f, 1), x)));
  const g = fr.map(() => 0);
  for (let n = 0; n < 800; n++) {
    const err = xs.map((x, j) => g.reduce((s, v, i) => s + v * basis[i][j], 0) - interp(c, x));
    g.forEach((_, i) => (g[i] = Math.min(MAX_DB, Math.max(-MAX_DB, g[i] - (0.5 * basis[i].reduce((s, b, j) => s + b * err[j], 0)) / xs.length))));
  }
  return g.map((v) => Math.round(v * 2) / 2 || 0);
}
</script>

<script setup lang="ts">
import { computed } from "vue";

// Presets are smooth target shapes; the band gains are fitted to them (on the default frequencies), so the
// result is the intended curve rather than ripples from overlapping bands.
const CURVES: Record<string, Curve> = {
  Flat: [[20, 0]],
  Bass: [[20, 6], [60, 6], [120, 4.5], [250, 1.5], [500, 0]],
  Treble: [[1000, 0], [2000, 1.5], [4000, 3.5], [8000, 5], [20000, 6]],
  Vokal: [[20, -2], [100, -1.5], [250, 0.5], [500, 2.5], [1000, 3.5], [2000, 3.5], [4000, 1.5], [8000, 0], [20000, -1]],
  Rock: [[20, 5], [80, 4.5], [200, 1.5], [500, -1], [1000, -1], [2000, 1], [4000, 3], [8000, 4], [20000, 4.5]],
};
// Harman uses its own band layout (shelves at its corners, peaks on its bumps); fits within ~0.7 dB.
const HARMAN_FREQS = [120, 40, 200, 400, 1500, 2500, 4000, 6000, 9000, 11000];
const PRESETS: Record<string, { freqs: number[]; gains: number[] }> = {
  ...Object.fromEntries(Object.entries(CURVES).map(([k, c]) => [k, { freqs: DEF_FREQS, gains: fit(c, DEF_FREQS) }])),
  Harman: { freqs: HARMAN_FREQS, gains: fit(HARMAN, HARMAN_FREQS, HARMAN_MAX) },
};
const sel = ref(0);

function preset(name: string) {
  freqs.value = PRESETS[name].freqs.slice();
  gains.value = PRESETS[name].gains.slice();
  apply();
}
const active = (name: string) =>
  freqs.value.every((f, i) => f === PRESETS[name].freqs[i]) && gains.value.every((v, i) => v === PRESETS[name].gains[i]);

// graph geometry (viewBox units)
const W = 1000, H = 300, L = 48, R = 20, T = 20, B = 34, RANGE = 14;
const x = (f: number) => L + (Math.log10(f / 20) / 3) * (W - L - R);
const fAt = (px: number) => 20 * 10 ** ((3 * (px - L)) / (W - L - R));
const y = (db: number) => T + ((RANGE - Math.max(-RANGE, Math.min(RANGE, db))) / (2 * RANGE)) * (H - T - B);
const dbAt = (py: number) => RANGE - ((py - T) / (H - T - B)) * 2 * RANGE;
const GRID_F = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];
const GRID_DB = [-12, -6, 0, 6, 12];
const fmtF = (f: number) => (f >= 1000 ? `${+(f / 1000).toFixed(f >= 10000 ? 0 : 1)}k` : `${Math.round(f)}`);
const fmtDb = (d: number) => `${d > 0 ? "+" : ""}${d.toFixed(1)}`;

const SAMPLES = Array.from({ length: 241 }, (_, k) => 20 * 1000 ** (k / 240));
const curve = computed(() => SAMPLES.map((f) => [x(f), y(response(gains.value, freqs.value, f))]));
const curvePath = computed(() => "M" + curve.value.map(([a, b]) => `${a.toFixed(1)},${b.toFixed(1)}`).join("L"));
const fillPath = computed(() => `${curvePath.value}L${W - R},${y(0)}L${L},${y(0)}Z`);
const harmanPath = "M" + SAMPLES.filter((f) => f <= HARMAN_MAX).map((f) => `${x(f).toFixed(1)},${y(harman(f)).toFixed(1)}`).join("L");
const preamp = computed(() => -Math.max(0, ...SAMPLES.map((f) => response(gains.value, freqs.value, f))));

const svg = ref<SVGSVGElement>();
let dragging = -1;
function toView(e: PointerEvent) {
  const r = svg.value!.getBoundingClientRect();
  return [((e.clientX - r.left) / r.width) * W, ((e.clientY - r.top) / r.height) * H];
}
function down(e: PointerEvent, i: number) {
  sel.value = dragging = i;
  (e.currentTarget as Element).setPointerCapture(e.pointerId);
}
function move(e: PointerEvent) {
  if (dragging < 0) return;
  const [px, py] = toView(e);
  const f = fAt(px);
  freqs.value[dragging] = Math.round(f / (f < 1000 ? 1 : 10)) * (f < 1000 ? 1 : 10);
  gains.value[dragging] = dbAt(py);
  apply();
}
function key(e: KeyboardEvent, i: number) {
  const step = { ArrowUp: [0, 0.5], ArrowDown: [0, -0.5], ArrowRight: [1, 0], ArrowLeft: [-1, 0] }[e.key];
  if (!step) return;
  e.preventDefault();
  sel.value = i;
  freqs.value[i] *= 2 ** (step[0] / 12);
  gains.value[i] += step[1];
  apply();
}
</script>

<template>
  <section class="panel eqz">
    <header class="head">
      <div class="title">
        <strong>Equalizer</strong>
        <span>Seret titik pada grafik untuk mengatur gain dan frekuensi, atau ketik nilainya di bawah.</span>
      </div>
      <span class="pre" title="Volume diturunkan sebesar puncak kurva supaya suara tidak pecah">Preamp {{ fmtDb(preamp) }} dB</span>
    </header>

    <div class="presets" role="group" aria-label="Preset">
      <button v-for="(_, name) in PRESETS" :key="name" class="chip" :class="{ sel: active(name), harman: name === 'Harman' }" @click="preset(name)">{{ name }}</button>
    </div>

    <div class="graph">
      <svg ref="svg" :viewBox="`0 0 ${W} ${H}`" @pointermove="move" @pointerup="dragging = -1" @pointercancel="dragging = -1">
        <g class="grid">
          <line v-for="f in GRID_F" :key="f" :x1="x(f)" :x2="x(f)" :y1="T" :y2="H - B" />
          <line v-for="d in GRID_DB" :key="d" :x1="L" :x2="W - R" :y1="y(d)" :y2="y(d)" :class="{ zero: d === 0 }" />
          <text v-for="f in GRID_F" :key="`t${f}`" :x="x(f)" :y="H - 10" text-anchor="middle">{{ fmtF(f) }}</text>
          <text v-for="d in GRID_DB" :key="`d${d}`" :x="L - 10" :y="y(d) + 4" text-anchor="end">{{ d > 0 ? `+${d}` : d }}</text>
        </g>
        <path class="harman-line" :d="harmanPath" />
        <path class="fill" :d="fillPath" />
        <path class="curve" :d="curvePath" />
        <g v-for="(f, i) in freqs" :key="i" class="node" :class="{ sel: sel === i }" :transform="`translate(${x(f)},${y(gains[i])})`"
          tabindex="0" role="slider" :aria-label="`Band ${i + 1}, ${fmtF(f)} Hz`" :aria-valuenow="gains[i]" :aria-valuemin="-12" :aria-valuemax="12"
          @pointerdown="down($event, i)" @keydown="key($event, i)" @focus="sel = i">
          <circle class="hit" r="22" />
          <circle class="dot" r="11" />
          <text y="4" text-anchor="middle">{{ i + 1 }}</text>
        </g>
      </svg>
      <div class="legend">
        <span><i class="sw eq-sw" />Kurva EQ</span>
        <span><i class="sw harman-sw" />Target Harman (panduan)</span>
      </div>
    </div>

    <div class="bands">
      <div v-for="(_, i) in freqs" :key="i" class="band" :class="{ sel: sel === i }" @click="sel = i">
        <div class="band-head">
          <span class="badge">{{ i + 1 }}</span>
          <span class="kind" :title="i === 0 ? 'Low shelf' : i === 9 ? 'High shelf' : 'Peak (Q 1.4)'">{{ i === 0 || i === 9 ? "Shelf" : "Peak" }}</span>
        </div>
        <label class="field gain">
          <span class="lbl">Gain</span>
          <input type="number" step="0.5" min="-12" max="12" v-model.number="gains[i]" @change="apply" @focus="sel = i" :aria-label="`Gain band ${i + 1} (dB)`" />
          <small>dB</small>
        </label>
        <label class="field">
          <span class="lbl">Frek</span>
          <input type="number" min="20" max="20000" v-model.number="freqs[i]" @change="apply" @focus="sel = i" :aria-label="`Frekuensi band ${i + 1} (Hz)`" />
          <small>Hz</small>
        </label>
      </div>
    </div>
  </section>
</template>

<style scoped>
.panel.eqz { max-width: none; min-width: 0; }
.head { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; flex-wrap: wrap; }
.title { display: flex; flex-direction: column; gap: 2px; min-width: 240px; flex: 1; }
.title strong { font-size: 20px; font-weight: 700; }
.title span { color: var(--on-surface-variant); font-size: 14px; }
.pre {
  padding: 8px 14px; border-radius: 16px; font-size: 13px; font-weight: 600; font-variant-numeric: tabular-nums;
  background: var(--surface-container-high); color: var(--on-surface-variant);
}

.presets { display: flex; flex-wrap: wrap; gap: 8px; }
.chip {
  height: 36px; padding: 0 16px; border: 1px solid var(--outline-variant); border-radius: 18px; background: transparent;
  font: inherit; font-size: 14px; font-weight: 600; color: var(--on-surface-variant); cursor: pointer;
  transition: border-radius 0.3s var(--spring), background-color 0.2s var(--emph), color 0.2s var(--emph);
}
.chip:active { border-radius: 12px; }
.chip.sel { border-color: transparent; background: var(--secondary-container); color: var(--on-secondary-container); }
.chip.harman:not(.sel) { border-style: dashed; border-color: var(--tertiary); color: var(--tertiary); }

.graph { border-radius: 20px; background: var(--surface-container); padding: 8px 8px 12px; }
svg { display: block; width: 100%; height: auto; touch-action: none; user-select: none; }
.grid line { stroke: var(--outline-variant); stroke-width: 1; opacity: 0.6; }
.grid line.zero { opacity: 1; stroke-width: 1.5; }
.grid text { fill: var(--on-surface-variant); font-size: 13px; font-variant-numeric: tabular-nums; }
.harman-line { fill: none; stroke: var(--tertiary); stroke-width: 2.5; stroke-dasharray: 8 7; stroke-linecap: round; }
.fill { fill: var(--primary); opacity: 0.14; }
.curve { fill: none; stroke: var(--primary); stroke-width: 3.5; stroke-linejoin: round; }
.node { cursor: grab; outline: none; }
.node:active { cursor: grabbing; }
.hit { fill: transparent; }
.dot { fill: var(--primary-container); stroke: var(--surface-container); stroke-width: 3; transition: r 0.3s var(--spring); }
.node text { fill: var(--on-primary-container); font-size: 12px; font-weight: 700; pointer-events: none; }
.node.sel .dot, .node:focus-visible .dot { fill: var(--primary); r: 14; }
.node.sel text, .node:focus-visible text { fill: var(--on-primary); }
.node:focus-visible .dot { stroke: var(--on-surface); }

.legend { display: flex; gap: 20px; padding: 4px 12px 0; font-size: 13px; color: var(--on-surface-variant); }
.legend span { display: inline-flex; align-items: center; gap: 8px; }
.sw { width: 22px; height: 0; border-top: 3px solid var(--primary); border-radius: 2px; }
.harman-sw { border-top: 3px dashed var(--tertiary); }

/* 10 across when there is room for a 5-digit frequency, otherwise two rows of 5 */
.panel.eqz { container-type: inline-size; }
.bands { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 8px; }
@container (min-width: 1000px) { .bands { grid-template-columns: repeat(10, minmax(0, 1fr)); } }
.band {
  display: flex; flex-direction: column; gap: 8px; padding: 10px 8px; border-radius: 20px; cursor: pointer;
  background: var(--surface-container); transition: background-color 0.2s var(--emph);
}
.band.sel { background: var(--secondary-container); }
.band-head { display: flex; align-items: center; justify-content: space-between; gap: 4px; padding: 0 2px; }
.badge {
  width: 22px; height: 22px; display: grid; place-items: center; border-radius: 11px; flex: none;
  font-size: 12px; font-weight: 700; background: var(--primary-container); color: var(--on-primary-container);
}
.band.sel .badge { background: var(--primary); color: var(--on-primary); }
.kind { font-size: 11px; font-weight: 600; color: var(--on-surface-variant); text-transform: uppercase; letter-spacing: 0.04em; }
.field {
  display: flex; align-items: baseline; gap: 3px; height: 32px; padding: 0 8px; border-radius: 10px; cursor: text;
  background: var(--surface-container-high); transition: background-color 0.2s var(--emph);
}
.field:focus-within { outline: 2px solid var(--primary); background: var(--surface-container-highest); }
.field input {
  flex: 1; width: 100%; min-width: 0; height: 100%; padding: 0; border: 0; outline: none; background: transparent; text-align: right;
  font: inherit; font-size: 13px; font-weight: 600; font-variant-numeric: tabular-nums; color: var(--on-surface);
  appearance: textfield; -moz-appearance: textfield;
}
.field.gain input { font-size: 16px; font-weight: 700; }
.field input::-webkit-inner-spin-button, .field input::-webkit-outer-spin-button { -webkit-appearance: none; margin: 0; }
.lbl { flex: none; font-size: 11px; font-weight: 600; color: var(--on-surface-variant); }
@container (min-width: 1000px) { .lbl { display: none; } }
.field small { width: 18px; flex: none; font-size: 11px; color: var(--on-surface-variant); }
</style>
