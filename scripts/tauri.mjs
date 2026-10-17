// `npm run tauri build` bumps the patch version first (set NO_BUMP=1 to skip; CI never bumps, the tag is the version).
import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const args = process.argv.slice(2);
if (args[0] === "build" && !process.env.CI && !process.env.NO_BUMP) {
  const edit = (file, from, to) => writeFileSync(file, readFileSync(file, "utf8").replace(from, to));
  const [maj, min, pat] = JSON.parse(readFileSync("package.json", "utf8")).version.split(".").map(Number);
  const next = `${maj}.${min}.${pat + 1}`;
  const prev = `${maj}.${min}.${pat}`;
  edit("package.json", `"version": "${prev}"`, `"version": "${next}"`);
  edit("package-lock.json", `"version": "${prev}"`, `"version": "${next}"`); // first match: the root package
  edit("src-tauri/tauri.conf.json", `"version": "${prev}"`, `"version": "${next}"`);
  edit("src-tauri/Cargo.toml", `version = "${prev}"`, `version = "${next}"`);
  console.log(`version ${prev} -> ${next}`);
}
process.exit(spawnSync("tauri", args, { stdio: "inherit", shell: true }).status ?? 1);
