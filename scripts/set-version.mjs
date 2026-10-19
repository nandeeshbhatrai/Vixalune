// Sets the application version everywhere it must agree: node scripts/set-version.mjs 0.2.1
import { readFileSync, writeFileSync } from "node:fs";

const v = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(v ?? "")) {
  console.error("usage: node scripts/set-version.mjs <major.minor.patch>");
  process.exit(2);
}
const json = (file, fn) => {
  const j = JSON.parse(readFileSync(file, "utf8"));
  fn(j);
  writeFileSync(file, JSON.stringify(j, null, 2) + "\n");
};
json("package.json", (j) => (j.version = v));
json("src-tauri/tauri.conf.json", (j) => (j.version = v));
json("package-lock.json", (j) => {
  j.version = v;
  if (j.packages?.[""]) j.packages[""].version = v;
});
let cargo = readFileSync("src-tauri/Cargo.toml", "utf8");
cargo = cargo.replace(/^version = "[^"]+"/m, `version = "${v}"`);
writeFileSync("src-tauri/Cargo.toml", cargo);
console.log(`version set to ${v} (also update CHANGELOG.md and run cargo update -p vixalune --offline)`);
