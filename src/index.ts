// The JS side of the codecs. Unlike `@voxelkloud/wasm-core`, this one has
// generated glue underneath it: the surface moves megabytes of typed arrays and
// every entry point can fail, which is the case wasm-bindgen exists for. What
// is hand-written here is the loading contract — the generated `init` wants a
// URL and a `fetch`, and the callers are a worker, a bundler and a Node test.

import init, {
  initSync,
  LazChunkDecoder,
  LazField,
  decodeLazFile,
  readLasEvlrs,
  readLasHeader,
} from "./generated/voxelkloud_wasm_codecs.js";
import type {
  DecodedLas,
  LasHeader,
  Vlr,
} from "./generated/voxelkloud_wasm_codecs.js";

export { LazChunkDecoder, LazField, decodeLazFile, readLasEvlrs, readLasHeader };
export type { DecodedLas, LasHeader, Vlr };

/** User id of the VLR that carries the laszip parameters. */
export const LASZIP_USER_ID = "laszip encoded";
/** Record id of the same. Its payload is what {@link LazChunkDecoder} takes. */
export const LASZIP_RECORD_ID = 22204;

/**
 * Where the `.wasm` sits next to this module.
 *
 * A bundler that understands `new URL(..., import.meta.url)` — esbuild, Vite,
 * webpack 5, Rollup — will emit the file and rewrite this to its hashed path.
 * One that does not will still resolve it at runtime relative to the bundle.
 */
export const wasmUrl = new URL(
  "./voxelkloud_wasm_codecs_bg.wasm",
  import.meta.url,
);

let ready: Promise<void> | undefined;

/**
 * Compile and instantiate the codecs. Call once before anything else here.
 *
 * Everything this module exports is a view onto one wasm instance, so this is a
 * process-wide switch rather than a handle you hold: calling it twice is a
 * no-op that returns the first result, and there is no way to run two isolated
 * instances in one JS realm. Run the codec in a worker — which the loader does
 * anyway, decoding off the main thread — and isolation is the worker's.
 *
 * @param source Compiled module, raw bytes, a `Response`, or a URL to fetch.
 *   Defaults to {@link wasmUrl}, which is right whenever the bundler kept the
 *   `.wasm` beside the JS.
 */
export function initLazCodec(
  source?: BufferSource | WebAssembly.Module | Response | URL | string,
): Promise<void> {
  // On failure the promise is dropped rather than cached: a fetch that lost the
  // network is worth retrying, and a caller that passed the wrong source should
  // be able to pass the right one.
  ready ??= instantiate(source ?? wasmUrl).catch((error: unknown) => {
    ready = undefined;
    throw error;
  });
  return ready;
}

async function instantiate(
  source: BufferSource | WebAssembly.Module | Response | URL | string,
): Promise<void> {
  // `initSync` is the only path that works in a Node test and in a worker with
  // the bytes already in hand; `init` is the only one that can fetch. Pick by
  // what was passed rather than making the caller pick.
  if (source instanceof WebAssembly.Module || isBufferSource(source)) {
    initSync({ module: source });
    return;
  }
  const bytes = await readFileUrl(source);
  if (bytes !== undefined) {
    initSync({ module: bytes });
    return;
  }
  await init({ module_or_path: source });
}

/**
 * Read a `file:` URL through Node, or return `undefined` to leave it to `init`.
 *
 * This exists because {@link wasmUrl} resolves to a `file:` URL under Node —
 * which is where a format driver's tests, a CLI and any SSR path all run — and
 * `fetch` refuses that scheme. Without it every Node caller has to know to read
 * the `.wasm` itself, which is a footgun disguised as a contract.
 *
 * The specifier is held in a variable on purpose: a static `import("node:fs")`
 * is something a browser bundler tries to resolve, and this branch is
 * unreachable in a browser. Any failure falls through to the fetch path so the
 * error the caller sees is the real one.
 */
async function readFileUrl(
  source: Response | URL | string,
): Promise<Uint8Array | undefined> {
  if (source instanceof Response) return undefined;
  const href = source instanceof URL ? source.href : source;
  if (!href.startsWith("file:")) return undefined;
  try {
    const specifier = "node:fs/promises";
    const fs = (await import(/* @vite-ignore */ specifier)) as {
      readFile(path: URL): Promise<Uint8Array>;
    };
    return await fs.readFile(new URL(href));
  } catch {
    return undefined;
  }
}

function isBufferSource(value: unknown): value is BufferSource {
  return value instanceof ArrayBuffer || ArrayBuffer.isView(value);
}

/**
 * Decode every node of an indexed file with one decoder.
 *
 * The reason this exists rather than a loop at the call site: building a
 * {@link LazChunkDecoder} parses the laszip VLR, and doing that per node is the
 * mistake the shape of the API should make hard. COPC and EPT both hand you a
 * hierarchy of `(bytes, pointCount)` and one VLR for the whole file.
 *
 * @param laszipRecord Payload of the `laszip encoded` VLR.
 * @param nodes Compressed chunks with the point count each one holds.
 * @param selection Optional field mask; see {@link LazField}.
 */
export function decodeLazChunks(
  laszipRecord: Uint8Array,
  nodes: Iterable<{ readonly data: Uint8Array; readonly pointCount: number }>,
  selection?: number,
): Uint8Array[] {
  const decoder = new LazChunkDecoder(laszipRecord);
  try {
    const out: Uint8Array[] = [];
    for (const node of nodes) {
      out.push(
        selection === undefined
          ? decoder.decode(node.data, node.pointCount)
          : decoder.decodeSelective(node.data, node.pointCount, selection),
      );
    }
    return out;
  } finally {
    // The decoder owns wasm memory that no GC of ours will reclaim.
    decoder.free();
  }
}
