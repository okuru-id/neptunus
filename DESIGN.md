# Design

PixelPlay Desktop uses **Material 3 Expressive**, matching PixelPlayer Android. The album art that is playing sets the theme.

## Colour

- All colour comes from Material roles that `src/lib/theme.ts` writes onto `:root` as `--primary`, `--secondary-container`, `--surface-container-high` and so on.
- The source colour is taken from the playing cover (`sourceColorFromImage`) and turned into a scheme with **SchemeVibrant**, which keeps the cover's hue. SchemeExpressive was rejected because it rotates the hue away from the cover.
- The fallback seed is `#ff6e40`. Dark or light follows the OS (`prefers-color-scheme`) and updates live.
- The roles are registered with `@property` so the whole UI fades to a new track's colours over 0.9s. This is the one authored motion moment.
- Usage:
  - primary: play button, filled buttons, wavy progress, current lyric line.
  - secondary-container: tonal buttons, active nav indicator, active song row, selected chips.
  - tertiary-container: brand mark.
  - primary-container: FAB, empty-state mark, "on" toggles.
- Never hardcode colours. The one exception is the S3 error status (`#8c1d18` / `#ffdad6`, the M3 error container), because the scheme has no error role wired yet.

## Type

- **Roboto Flex** (variable, self-hosted via `@fontsource-variable/roboto-flex/full.css`).
- Display: weight 800, `wdth` 115–120, tracking -0.03em. Used for view titles (40–64px), the Now Playing title, and lyrics.
- UI: 12–18px, weight 500–650. Use tabular numbers for times.

## Shape

- Album art cycles through `ART_SHAPES` in `src/lib/shapes.ts`: cookie9, squircle, clover4, arch, sunny8, circle. Polar clip paths are defined once as SVG `clipPath` elements in the page.
- Artists are circles. List art uses a 12px radius, the player art 16px, and cards/panels 28px.
- Buttons are pills, and on press they squish to a 12px radius (`--spring`).
- The play button morphs from a pill (paused) to an 18px squircle (playing).
- The Now Playing cover is a slowly rotating cookie12 with the image counter-rotated so it stays upright. It pauses with playback.

## Components

- **Navigation drawer** (`--sidebar` 248px, 216px at ≤1100px): extended FAB "Tambah folder", and 5 destinations as 56px-tall full-width pills laid out as [icon][label]. Playlist shows a count badge, and the active pill uses `secondary-container`.
- **Floating player bar**: full window width (16px inset each side, also below the drawer), radius 28, `surface-container-high`.
  - A surface-coloured spread shadow hides list rows scrolling past it.
  - Contents: art and title (opens Now Playing), transport group, wavy progress (`src/lib/WavyProgress.vue`), lyrics toggle, and a 16px-track M3 slider for volume.
- **Wavy progress**: the played part is a sine wave in primary that rolls while playing and settles flat on pause. The remaining track is a flat secondary-container line with an end stop dot. An invisible range input on top handles seeking.
- **Icons**: Material Symbols Rounded (filled) from `@material-symbols/svg-400`, inlined through `src/lib/Icon.vue` (`currentColor`). Never use emoji or Unicode glyphs as icons.

## Motion

- `--spring: cubic-bezier(0.3, 1.3, 0.5, 1)` is a mild overshoot for spatial changes: shape morphs, press squish, tile hover.
- `--emph: cubic-bezier(0.2, 0, 0, 1)` is used for colour and opacity.
- Only animate transforms, radius, colour and opacity. Never animate width or height.
- `prefers-reduced-motion` stops the rotating shapes, the equaliser bars and the wave phase.

## Layout

- Up to 1100px wide: the volume area is hidden and Now Playing becomes a single column without lyrics.
- The minimum window size is 960×640 (set in `tauri.conf.json`).

## Window frame

- The system frame is off (`decorations: false`, `transparent: true`, `shadow: false` in `tauri.conf.json`).
- The body draws the window:
  - `--frame-radius` 16px (M3 large corner) with `contain: paint`, so fixed layers are clipped too.
  - A 1px `outline-variant` hairline sits on top via `body::after`.
  - When maximized, `html.maximized` sets the radius to 0 and hides the hairline.
- `src/lib/TitleBar.vue` is a 48px drag region (`data-tauri-drag-region`, double-click maximizes). It carries the brand (30px cookie mark + "PixelPlay", `pointer-events: none` so it still drags) on the left, level with the M3 pill window controls on the right: minimize, maximize/restore (outlined symbols), and close (hover turns error red `#b3261e`).
- **Frame and sheet**: the window background (`--frame-bg`) is darker than the content: `surface-container-lowest` in dark mode, `surface-container-high` in light. The titlebar sits directly on it.
  - The drawer and main area live in one rounded content sheet (`.app`, `surface` background) with only the top corners rounded (`--sheet-radius` 20px). It runs flush to the window edges on the sides and bottom (`--sheet-gap` 0), where the window frame radius takes over.
  - Now Playing uses the same sheet geometry.
  - The player's masking spread is reduced by `--sheet-gap` so it stays inside the sheet.
