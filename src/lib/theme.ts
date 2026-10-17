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
] as const;

export const DEFAULT_SEED = argbFromHex("#ff6e40");

const dark = matchMedia("(prefers-color-scheme: dark)");
let seed = DEFAULT_SEED;

const kebab = (s: string) => s.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());

/** Writes a Material 3 vibrant scheme (keeps the cover hue) for `source` onto :root as --role custom properties. */
export function applyTheme(source = seed) {
  seed = source;
  const scheme = new SchemeVibrant(Hct.fromInt(source), dark.matches, 0);
  const root = document.documentElement;
  for (const role of ROLES) root.style.setProperty("--" + kebab(role), hexFromArgb(scheme[role]));
  root.style.colorScheme = dark.matches ? "dark" : "light";
}

dark.addEventListener("change", () => applyTheme());

export async function seedFromImage(src: string): Promise<number> {
  const img = new Image();
  img.src = src;
  await img.decode();
  return sourceColorFromImage(img);
}
