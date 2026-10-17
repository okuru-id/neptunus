// Material 3 Expressive shape library, approximated as polar curves in a 0..1 box
// so they can be used as `clipPathUnits="objectBoundingBox"` clip paths.
function polar(lobes: number, depth: number, steps = 240) {
  let d = "";
  for (let i = 0; i <= steps; i++) {
    const t = (i / steps) * Math.PI * 2;
    const r = (0.5 * (1 + depth * Math.cos(lobes * t))) / (1 + depth);
    const x = 0.5 + r * Math.cos(t - Math.PI / 2);
    const y = 0.5 + r * Math.sin(t - Math.PI / 2);
    d += `${i ? "L" : "M"}${x.toFixed(4)} ${y.toFixed(4)}`;
  }
  return d + "Z";
}

export const CLIP_SHAPES = {
  cookie9: polar(9, 0.07),
  cookie12: polar(12, 0.045),
  clover4: polar(4, 0.16),
  sunny8: polar(8, 0.05),
};

/** Cycle used for album art; css shapes are done with border-radius. */
export const ART_SHAPES = ["cookie9", "squircle", "clover4", "arch", "sunny8", "circle"] as const;
export type ArtShape = (typeof ART_SHAPES)[number];

export const isClip = (s: string): s is keyof typeof CLIP_SHAPES => s in CLIP_SHAPES;
