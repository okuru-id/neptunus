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

const kebab = (s: string) => s.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());

/** Writes a Material 3 vibrant scheme (keeps the cover hue) for `source` onto :root as --role custom properties. */
export function applyTheme(source = seed) {
  seed = source;
  const scheme = new SchemeVibrant(Hct.fromInt(source), isDark(), 0);
  const root = document.documentElement;
  for (const role of ROLES) root.style.setProperty("--" + kebab(role), hexFromArgb(scheme[role]));
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
