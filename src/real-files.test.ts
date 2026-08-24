// Conformance against files this repo did not write.
//
// `codec.test.ts` round-trips through laz-rs's own compressor, which proves the
// wasm boundary and the framing but cannot prove laszip conformance — the same
// library on both ends would agree with itself about a wrong answer. These
// files came out of PotreeConverter, Entwine and untwine, over LASzip's C++
// implementation, and one of them ships an uncompressed twin that settles the
// question byte for byte.
//
// They live under `demo/potree/`, which is gitignored: too large to commit and
// rebuildable from `demo/data/fetch-*.sh`. So every test here skips when the
// files are absent, and says so.

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";
import {
  LASZIP_RECORD_ID,
  LASZIP_USER_ID,
  LazChunkDecoder,
  decodeLazFile,
  initLazCodec,
  readLasEvlrs,
  readLasHeader,
} from "./index.js";

const WASM = new URL("../dist/voxelkloud_wasm_codecs_bg.wasm", import.meta.url);
const CLOUDS = new URL("../../../demo/potree/pointclouds/", import.meta.url);

const LAS_DIR = new URL("lion_takanawa_las/data/", CLOUDS);
const LAZ_DIR = new URL("lion_takanawa_laz/data/", CLOUDS);
const COPC = new URL("lion_takanawa.copc.laz", CLOUDS);
const EPT_NODE = new URL(
  "lion_takanawa_ept_laz/ept-data/0-0-0-0.laz",
  CLOUDS,
);

const HAS_PAIRS = existsSync(LAS_DIR) && existsSync(LAZ_DIR);
const HAS_COPC = existsSync(COPC);
const HAS_EPT = existsSync(EPT_NODE);

if (!(HAS_PAIRS && HAS_COPC && HAS_EPT)) {
  console.warn(
    "@voxelkloud/wasm-codecs: demo/potree/pointclouds is missing or partial, " +
      "so the conformance tests against third-party LAZ are skipped. Fetch it " +
      "with demo/data/fetch-large.sh.",
  );
}

beforeAll(async () => {
  await initLazCodec(readFileSync(fileURLToPath(WASM)));
});

/** The point block of an uncompressed LAS file. */
function lasPoints(bytes: Uint8Array): Uint8Array {
  const header = readLasHeader(bytes);
  const start = header.offsetToPointData;
  const end = start + header.pointCount * header.pointSize;
  header.free();
  return bytes.subarray(start, end);
}

describe.skipIf(!HAS_PAIRS)("PotreeConverter LAS/LAZ pairs", () => {
  // Every octree node PotreeConverter wrote twice, once each way. Same points,
  // same order, one compressed by LASzip's C++ encoder: byte equality is the
  // whole conformance claim in one assertion, 200 times over.
  // Guarded, even though the `describe` above is already skipped when the data
  // is missing: vitest evaluates a describe BODY to collect its tests and only
  // then honours `skipIf`, so an eager `readdirSync` throws before the skip can
  // take effect. The intent was right and the placement defeated it — which is
  // how a repository with no test data still failed on the missing directory.
  const names = HAS_PAIRS
    ? readdirSync(LAZ_DIR)
        .filter((n) => n.endsWith(".laz"))
        .sort()
    : [];

  it("has pairs to compare", () => {
    expect(names.length).toBeGreaterThan(100);
  });

  it("decodes every node to the bytes of its uncompressed twin", () => {
    let points = 0;
    for (const name of names) {
      const las = new URL(name.replace(/\.laz$/, ".las"), LAS_DIR);
      if (!existsSync(las)) continue;

      const decoded = decodeLazFile(
        new Uint8Array(readFileSync(new URL(name, LAZ_DIR))),
      );
      const expected = lasPoints(new Uint8Array(readFileSync(las)));
      // One assertion per node, named, so a failure says which node.
      expect(
        Buffer.compare(
          Buffer.from(
            decoded.points.buffer,
            decoded.points.byteOffset,
            decoded.points.byteLength,
          ),
          Buffer.from(expected.buffer, expected.byteOffset, expected.byteLength),
        ),
        `${name} did not decode to ${name.replace(/\.laz$/, ".las")}`,
      ).toBe(0);
      points += decoded.pointCount;
      decoded.free();
    }
    expect(points).toBeGreaterThan(300_000);
  });
});

describe.skipIf(!HAS_EPT)("an Entwine EPT node", () => {
  it("decodes to coordinates that fill the header's own bounding box", () => {
    // No uncompressed twin here, so the oracle is the writer's own bbox: it was
    // computed from the true points, and a decode that drifted by one point
    // would not reproduce both extremes on all three axes.
    const decoded = decodeLazFile(
      new Uint8Array(readFileSync(fileURLToPath(EPT_NODE))),
    );
    const header = decoded.header;
    const { pointSize } = header;
    const scale = Array.from(header.scale);
    const offset = Array.from(header.offset);
    const min = Array.from(header.min);
    const max = Array.from(header.max);

    const points = decoded.points;
    const view = new DataView(
      points.buffer,
      points.byteOffset,
      points.byteLength,
    );
    const lo = [Infinity, Infinity, Infinity];
    const hi = [-Infinity, -Infinity, -Infinity];
    for (let i = 0; i < decoded.pointCount; i++) {
      for (let axis = 0; axis < 3; axis++) {
        const v =
          view.getInt32(i * pointSize + axis * 4, true) * scale[axis]! +
          offset[axis]!;
        lo[axis] = Math.min(lo[axis]!, v);
        hi[axis] = Math.max(hi[axis]!, v);
      }
    }

    for (let axis = 0; axis < 3; axis++) {
      // Within one scale step: the header stores the bounds as doubles built
      // from the same scaled integers.
      expect(lo[axis]!).toBeCloseTo(min[axis]!, 6);
      expect(hi[axis]!).toBeCloseTo(max[axis]!, 6);
    }
    header.free();
    decoded.free();
  });
});

describe.skipIf(!HAS_COPC)("a COPC file", () => {
  const bytes = () => new Uint8Array(readFileSync(fileURLToPath(COPC)));

  it("reads its header and VLRs from the first 4 KiB alone", () => {
    // What a driver does over HTTP: one ranged GET, then decide.
    const header = readLasHeader(bytes().subarray(0, 4096));
    expect(header.version).toBe("1.4");
    expect(header.pointFormat).toBe(7);
    expect(header.pointSize).toBe(40);
    expect(header.pointCount).toBeGreaterThan(300_000);
    expect(header.compressed).toBe(true);
    expect(header.vlrsComplete).toBe(true);

    // COPC's own VLR must be first, and the laszip VLR must be present — the
    // two facts the COPC driver keys off.
    const ids = header.vlrs.map((v) => {
      const id = `${v.userId}:${v.recordId}`;
      v.free();
      return id;
    });
    expect(ids[0]).toBe("copc:1");
    expect(ids).toContain(`${LASZIP_USER_ID}:${LASZIP_RECORD_ID}`);

    const copc = header.findVlr("copc", 1)!;
    expect(copc.data).toHaveLength(160);
    copc.free();
    expect(header.evlrCount).toBe(1);
    expect(header.evlrOffset).toBeGreaterThan(0);
    header.free();
  });

  it("reads the hierarchy EVLR from the tail of the file", () => {
    const file = bytes();
    const header = readLasHeader(file.subarray(0, 4096));
    const evlrs = readLasEvlrs(
      file.subarray(header.evlrOffset),
      header.evlrCount,
    );
    header.free();

    expect(evlrs).toHaveLength(1);
    expect(evlrs[0]!.userId).toBe("copc");
    expect(evlrs[0]!.recordId).toBe(1000);
    // A hierarchy page is a whole number of 32-byte entries.
    expect(evlrs[0]!.data.length % 32).toBe(0);
    evlrs[0]!.free();
  });

  it("decodes the root node's chunk", () => {
    const file = bytes();
    const header = readLasHeader(file.subarray(0, 4096));

    // Reading the COPC info VLR and walking its hierarchy is the driver's job
    // in Task B2, not this package's. It is done by hand here to prove the one
    // thing B2 depends on: a COPC node is a bare laszip chunk, and this decoder
    // takes it as it lies on disk.
    const info = header.findVlr("copc", 1)!;
    const infoView = new DataView(
      info.data.buffer,
      info.data.byteOffset,
      info.data.byteLength,
    );
    const center = [0, 8, 16].map((at) => infoView.getFloat64(at, true));
    const halfSize = infoView.getFloat64(24, true);
    const rootHierOffset = Number(infoView.getBigUint64(40, true));
    const rootHierSize = Number(infoView.getBigUint64(48, true));
    info.free();

    const page = file.subarray(rootHierOffset, rootHierOffset + rootHierSize);
    const pageView = new DataView(
      page.buffer,
      page.byteOffset,
      page.byteLength,
    );
    // Entries are 32 bytes: a four-int key, then offset, byte size, point count.
    let root:
      | { offset: number; byteSize: number; pointCount: number }
      | undefined;
    for (let at = 0; at + 32 <= page.length; at += 32) {
      const level = pageView.getInt32(at, true);
      const x = pageView.getInt32(at + 4, true);
      const y = pageView.getInt32(at + 8, true);
      const z = pageView.getInt32(at + 12, true);
      if (level === 0 && x === 0 && y === 0 && z === 0) {
        root = {
          offset: Number(pageView.getBigUint64(at + 16, true)),
          byteSize: pageView.getInt32(at + 24, true),
          pointCount: pageView.getInt32(at + 28, true),
        };
        break;
      }
    }
    expect(root).toBeDefined();
    expect(root!.pointCount).toBeGreaterThan(0);

    const laszip = header.findVlr(LASZIP_USER_ID, LASZIP_RECORD_ID)!;
    const decoder = new LazChunkDecoder(laszip.data);
    laszip.free();
    // COPC writes one variable-size chunk per node.
    expect(decoder.chunkSize).toBe(0);
    expect(decoder.pointSize).toBe(header.pointSize);

    const records = decoder.decode(
      file.subarray(root!.offset, root!.offset + root!.byteSize),
      root!.pointCount,
    );
    decoder.free();
    expect(records).toHaveLength(root!.pointCount * header.pointSize);

    // Every point must land inside the octree node the hierarchy claimed, which
    // for the root is the whole cube. Half a metre of scatter is what a decode
    // that lost sync produces; the cube is 5.7 m across, so this catches it.
    //
    // The tolerance is one scale step, not zero: this writer derived the cube
    // from unquantised source bounds, so its faces sit off the integer grid the
    // points are stored on and a legitimate extreme point rounds just past
    // them.
    const scale = Array.from(header.scale);
    const offset = Array.from(header.offset);
    const view = new DataView(
      records.buffer,
      records.byteOffset,
      records.byteLength,
    );
    const lo = [Infinity, Infinity, Infinity];
    const hi = [-Infinity, -Infinity, -Infinity];
    for (let i = 0; i < root!.pointCount; i++) {
      for (let axis = 0; axis < 3; axis++) {
        const v =
          view.getInt32(i * header.pointSize + axis * 4, true) * scale[axis]! +
          offset[axis]!;
        lo[axis] = Math.min(lo[axis]!, v);
        hi[axis] = Math.max(hi[axis]!, v);
      }
    }
    for (let axis = 0; axis < 3; axis++) {
      const eps = scale[axis]!;
      expect(lo[axis]!).toBeGreaterThanOrEqual(center[axis]! - halfSize - eps);
      expect(hi[axis]!).toBeLessThanOrEqual(center[axis]! + halfSize + eps);
    }
    header.free();
  });
});
