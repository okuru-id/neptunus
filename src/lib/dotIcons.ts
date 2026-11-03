// The Nothing style's dot-matrix icons: each drawn on a 9×9 grid, every lit cell a whole round dot (Icon.vue
// renders them as SVG circles, so they stay round at any size). The navigation ones are outlines, like the line icons
// they stand in for; the rest keep their line icons, which read better small.

const GRIDS = {
  catalog: [
    "..XXXXX..",
    ".X.....X.",
    "X.......X",
    "X...X...X",
    "X..XXX..X",
    "X...X...X",
    "X.......X",
    ".X.....X.",
    "..XXXXX..",
  ],
  note: [
    "....XXXX.",
    "....X..X.",
    "....X..X.",
    "....X....",
    "....X....",
    "..XXX....",
    ".X..X....",
    ".X..X....",
    "..XX.....",
  ],
  album: [
    "..XXXXX..",
    ".X.....X.",
    "X.......X",
    "X..XXX..X",
    "X..X.X..X",
    "X..XXX..X",
    "X.......X",
    ".X.....X.",
    "..XXXXX..",
  ],
  artist: [
    "...XXX...",
    "..X...X..",
    "..X...X..",
    "..X...X..",
    "...XXX...",
    ".........",
    "..XXXXX..",
    ".X.....X.",
    "X.......X",
  ],
  playlist: [
    ".........",
    "XXXXXX...",
    ".........",
    "XXXXXX...",
    ".......X.",
    "XXXX...X.",
    ".......X.",
    ".....XXX.",
    ".....XXX.",
  ],
  movie: [
    ".........",
    "XXXXXXXXX",
    "X.X.X.X.X",
    "XXXXXXXXX",
    "X.......X",
    "X.......X",
    "X.......X",
    "XXXXXXXXX",
    ".........",
  ],
  play: [
    ".........",
    "..X......",
    "..XX.....",
    "..XXX....",
    "..XXXX...",
    "..XXX....",
    "..XX.....",
    "..X......",
    ".........",
  ],
  pause: [
    ".........",
    "..XX.XX..",
    "..XX.XX..",
    "..XX.XX..",
    "..XX.XX..",
    "..XX.XX..",
    "..XX.XX..",
    "..XX.XX..",
    ".........",
  ],
  next: [
    ".........",
    ".X....X..",
    ".XX...X..",
    ".XXX..X..",
    ".XXXX.X..",
    ".XXX..X..",
    ".XX...X..",
    ".X....X..",
    ".........",
  ],
  prev: [
    ".........",
    "..X....X.",
    "..X...XX.",
    "..X..XXX.",
    "..X.XXXX.",
    "..X..XXX.",
    "..X...XX.",
    "..X....X.",
    ".........",
  ],
} as const;

export type DotIconName = keyof typeof GRIDS;

/** The icon as SVG: one circle per lit cell, a dot a bit narrower than its cell so neighbours stay apart. */
function svg(rows: readonly string[]) {
  const dots = rows.flatMap((row, y) => [...row].flatMap((c, x) => (c === "X" ? [`<circle cx="${x + 0.5}" cy="${y + 0.5}" r="0.38"/>`] : [])));
  return `<svg viewBox="0 0 9 9" xmlns="http://www.w3.org/2000/svg">${dots.join("")}</svg>`;
}

export const dotIcons: Partial<Record<string, string>> = Object.fromEntries(
  Object.entries(GRIDS).map(([name, rows]) => [name, svg(rows)]),
);
