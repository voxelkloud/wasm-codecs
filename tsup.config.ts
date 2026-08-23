import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm"],
  dts: true,
  sourcemap: true,
  // The generated bindgen glue is bundled in; only the `.wasm` stays a separate
  // file, and `build:wasm` has already copied it into dist.
  clean: false,
});
