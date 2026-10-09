// Builds the `cms` CLI and stages it as a Tauri sidecar:
//   src-tauri/binaries/cms-<target-triple>[.exe]
//
// Only used with the bundle overlay (src-tauri/tauri.bundle.conf.json). The
// overlay is NOT part of tauri.conf.json on purpose: `externalBin` makes
// tauri-build require the staged file at compile time of EVERY bin, which
// would break `cargo test` / `clippy` and the very build of `cms` below.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const tauriDir = join(root, "src-tauri");

function hostTriple() {
  // `--print host-tuple` is stable since Rust 1.84; older toolchains only
  // expose the triple through `-vV`.
  try {
    return execFileSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" }).trim();
  } catch {
    const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
    const m = out.match(/^host:\s*(\S+)/m);
    if (!m) throw new Error("could not determine the host target triple from rustc -vV");
    return m[1];
  }
}

const triple = process.env.TAURI_ENV_TARGET_TRIPLE || hostTriple();
const ext = triple.includes("windows") ? ".exe" : "";

console.log(`[build-sidecar] building cms for ${triple}`);
// `tauri build` exports the merged config (including the overlay's externalBin)
// as TAURI_CONFIG; if the cargo child inherited it, tauri-build would demand
// the very file this script is producing. Build cms with the base config only.
const { TAURI_CONFIG: _omit, ...cleanEnv } = process.env;
execFileSync(
  "cargo",
  ["build", "--release", "--bin", "cms", "--target", triple],
  { cwd: tauriDir, stdio: "inherit", env: cleanEnv },
);

const src = join(tauriDir, "target", triple, "release", `cms${ext}`);
const destDir = join(tauriDir, "binaries");
const dest = join(destDir, `cms-${triple}${ext}`);
mkdirSync(destDir, { recursive: true });
copyFileSync(src, dest);
console.log(`[build-sidecar] staged ${dest}`);
