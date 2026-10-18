// `npm run tauri build` bumps the patch version first (set NO_BUMP=1 to skip; CI never bumps, the tag is the version).
// `npm run tauri build -- --version=0.2.1` sets that version instead (the `--` is needed: npm keeps `--version` for itself).
import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const args = process.argv.slice(2);
const help = args.includes("--help") || args.includes("-h");
if (help) {
  console.log(`Neptunus wrapper around the tauri CLI (scripts/tauri.mjs)

  npm run tauri build                          bump the patch version, then build
  npm run tauri build -- --version=X.Y.Z       set version X.Y.Z, then build
  NO_BUMP=1 npm run tauri build                build without touching the version

  Options go after \`--\` (npm keeps --version for itself). Tauri's own help follows.
`);
}
const at = args.findIndex((a) => a === "--version" || a.startsWith("--version="));
let wanted;
if (at >= 0) {
  wanted = args[at].includes("=") ? args[at].split("=")[1] : args[at + 1];
  args.splice(at, args[at].includes("=") ? 1 : 2);
  if (!/^\d+\.\d+\.\d+$/.test(wanted ?? "")) {
    console.error(`--version: expected x.y.z, got "${wanted ?? ""}"`);
    process.exit(1);
  }
}

if (args[0] === "build" && !help && !process.env.CI && (wanted || !process.env.NO_BUMP)) {
  const edit = (file, from, to) => writeFileSync(file, readFileSync(file, "utf8").replace(from, to));
  const prev = JSON.parse(readFileSync("package.json", "utf8")).version;
  const [maj, min, pat] = prev.split(".").map(Number);
  const next = wanted ?? `${maj}.${min}.${pat + 1}`;
  edit("package.json", `"version": "${prev}"`, `"version": "${next}"`);
  edit("package-lock.json", `"version": "${prev}"`, `"version": "${next}"`); // first match: the root package
  edit("src-tauri/tauri.conf.json", `"version": "${prev}"`, `"version": "${next}"`);
  edit("src-tauri/Cargo.toml", `version = "${prev}"`, `version = "${next}"`);
  console.log(`version ${prev} -> ${next}`);
}
process.exit(spawnSync("tauri", args, { stdio: "inherit", shell: true }).status ?? 1);
