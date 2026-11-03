import { ref } from "vue";
import {
  Hct,
  SchemeVibrant,
  argbFromHex,
  hexFromArgb,
  sourceColorFromImage,
} from "@material/material-color-utilities";

const ROLES = [
  "primary", "onPrimary", "primaryContainer", "onPrimaryContainer",
  "secondary", "secondaryContainer", "onSecondaryContainer",
  "tertiary", "tertiaryContainer", "onTertiaryContainer",
  "surface", "surfaceContainerLowest", "surfaceContainerLow", "surfaceContainer",
  "surfaceContainerHigh", "surfaceContainerHighest",
  "onSurface", "onSurfaceVariant", "outline", "outlineVariant",
  "error", "onError", "errorContainer", "onErrorContainer",
] as const;

// a calm blue while no song is playing (the cover colours the app otherwise)
export const DEFAULT_SEED = argbFromHex("#4a7fc1");

const dark = matchMedia("(prefers-color-scheme: dark)");
let seed = DEFAULT_SEED;

/** Light or dark: the system's, or one picked in Settings › Tampilan. */
export type ThemeMode = "system" | "light" | "dark";
const MODE_KEY = "neptunus.theme";
const stored = (() => { try { return localStorage.getItem(MODE_KEY); } catch { return null; } })();
export const themeMode = ref<ThemeMode>(stored === "light" || stored === "dark" ? stored : "system");
const isDark = () => (themeMode.value === "system" ? dark.matches : themeMode.value === "dark");

export function setThemeMode(mode: ThemeMode) {
  themeMode.value = mode;
  try { localStorage.setItem(MODE_KEY, mode); } catch {}
  applyTheme();
}

/** The look: Material 3 Expressive, coloured from the cover, or Nothing's (nothing.tech): black, grey and
 *  white, dot-matrix type, a red kept for signals. Settings › Tampilan; styles under `html.nothing`. */
export type UiStyle = "material" | "nothing";
const STYLE_KEY = "neptunus.style";
const storedStyle = (() => { try { return localStorage.getItem(STYLE_KEY); } catch { return null; } })();
export const uiStyle = ref<UiStyle>(storedStyle === "nothing" ? "nothing" : "material");

export function setUiStyle(style: UiStyle) {
  uiStyle.value = style;
  try { localStorage.setItem(STYLE_KEY, style); } catch {}
  applyTheme();
}

// Nothing's palette on the Material roles the UI is written against: the greys are the hierarchy, active
// things invert black and white (primary is white on dark), and nothing takes the cover's colour.
type Palette = Record<(typeof ROLES)[number], string>;
const NOTHING: { dark: Palette; light: Palette } = {
  // the redesign brief: #000 background, #0A0A0A secondary, #151515 surface, #1C1C1C elevated, text #FFF /
  // #888 / #555, borders #292929
  dark: {
    primary: "#FFFFFF", onPrimary: "#000000", primaryContainer: "#151515", onPrimaryContainer: "#FFFFFF",
    secondary: "#888888", secondaryContainer: "#1C1C1C", onSecondaryContainer: "#FFFFFF",
    tertiary: "#FFFFFF", tertiaryContainer: "#151515", onTertiaryContainer: "#FFFFFF",
    surface: "#000000", surfaceContainerLowest: "#000000", surfaceContainerLow: "#0A0A0A", surfaceContainer: "#0A0A0A",
    surfaceContainerHigh: "#151515", surfaceContainerHighest: "#1C1C1C",
    onSurface: "#FFFFFF", onSurfaceVariant: "#888888", outline: "#555555", outlineVariant: "#292929",
    error: "#E82027", onError: "#FFFFFF", errorContainer: "#2A0B09", onErrorContainer: "#FF4A50",
  },
  light: {
    primary: "#000000", onPrimary: "#FFFFFF", primaryContainer: "#EDEDED", onPrimaryContainer: "#000000",
    secondary: "#585A5A", secondaryContainer: "#E5E7EB", onSecondaryContainer: "#1C1C1C",
    tertiary: "#1C1C1C", tertiaryContainer: "#EDEDED", onTertiaryContainer: "#000000",
    surface: "#F2F2F2", surfaceContainerLowest: "#FFFFFF", surfaceContainerLow: "#FFFFFF", surfaceContainer: "#FFFFFF",
    surfaceContainerHigh: "#EDEDED", surfaceContainerHighest: "#E5E7EB",
    onSurface: "#1C1C1C", onSurfaceVariant: "#585A5A", outline: "#CFCFCF", outlineVariant: "#E5E7EB",
    error: "#D23B30", onError: "#FFFFFF", errorContainer: "#FBE4E2", onErrorContainer: "#C2141C",
  },
};

const kebab = (s: string) => s.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());

/** Writes a Material 3 vibrant scheme (keeps the cover hue) for `source` onto :root as --role custom
 *  properties; in the Nothing style, its fixed palette instead (the cover is still remembered for later). */
export function applyTheme(source = seed) {
  seed = source;
  const root = document.documentElement;
  const nothing = uiStyle.value === "nothing";
  root.classList.toggle("nothing", nothing);
  if (nothing) {
    const palette = NOTHING[isDark() ? "dark" : "light"];
    for (const role of ROLES) root.style.setProperty("--" + kebab(role), palette[role]);
  } else {
    const scheme = new SchemeVibrant(Hct.fromInt(source), isDark(), 0);
    for (const role of ROLES) root.style.setProperty("--" + kebab(role), hexFromArgb(scheme[role]));
  }
  root.style.colorScheme = isDark() ? "dark" : "light";
  root.dataset.theme = isDark() ? "dark" : "light";
}

dark.addEventListener("change", () => applyTheme());

export async function seedFromImage(src: string): Promise<number> {
  const img = new Image();
  img.src = src;
  await img.decode();
  return sourceColorFromImage(img);
}
