// Run a command only if the wasm build output is there; otherwise say why and
// succeed.
//
// The rest of the repo builds without a Rust toolchain, and `@voxelkloud/
// wasm-core` keeps that true by skipping its tests from inside vitest. This
// package cannot: `src/index.ts` imports the generated bindgen glue at module
// scope, so a missing build throws on import, before any `skipIf` runs. The
// guard has to sit outside the process.

import { existsSync } from "node:fs";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT = new URL("../", import.meta.url);
const REQUIRED = [
  "src/generated/voxelkloud_wasm_codecs.js",
  "dist/voxelkloud_wasm_codecs_bg.wasm",
];

const missing = REQUIRED.filter((p) => !existsSync(new URL(p, ROOT)));
if (missing.length > 0) {
  console.warn(
    `@voxelkloud/wasm-codecs: skipping \`${process.argv.slice(2).join(" ")}\` — ` +
      `${missing.join(", ")} ${missing.length === 1 ? "is" : "are"} missing.\n` +
      "Build with `pnpm --filter @voxelkloud/wasm-codecs build`, which needs " +
      "cargo, `rustup target add wasm32-unknown-unknown`, and `cargo install " +
      "wasm-bindgen-cli --version 0.2.127`.",
  );
  process.exit(0);
}

const [command, ...args] = process.argv.slice(2);
spawn(command, args, {
  stdio: "inherit",
  cwd: fileURLToPath(ROOT),
  shell: process.platform === "win32",
}).on("exit", (code, signal) => {
  process.exit(signal ? 1 : (code ?? 1));
});
