import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";
import {
  LASZIP_RECORD_ID,
  LASZIP_USER_ID,
  LazChunkDecoder,
  LazField,
  decodeLazChunks,
  decodeLazFile,
  initLazCodec,
  readLasHeader,
} from "./index.js";

// `scripts/with-wasm.mjs` gates the whole run on the build being there, so by
// the time vitest starts the module exists.
const WASM = new URL("../dist/voxelkloud_wasm_codecs_bg.wasm", import.meta.url);

const FIXTURES = new URL("../fixtures/", import.meta.url);

function fixture(name: string): Uint8Array {
  return new Uint8Array(readFileSync(new URL(name, FIXTURES)));
}

/** The point block of an uncompressed fixture: the oracle for its LAZ twin. */
function lasPoints(name: string): Uint8Array {
  const las = fixture(`${name}.las`);
  const header = readLasHeader(las);
  const start = header.offsetToPointData;
  const end = start + header.pointCount * header.pointSize;
  header.free();
  return las.subarray(start, end);
}

/** Every fixture the generator writes, and the LAS version each implies. */
const CASES = [
  { name: "fmt0", format: 0, size: 20, version: "1.2" },
  { name: "fmt2", format: 2, size: 26, version: "1.2" },
  { name: "fmt3", format: 3, size: 34, version: "1.2" },
  { name: "fmt3-extra4", format: 3, size: 38, version: "1.2" },
  { name: "fmt6", format: 6, size: 30, version: "1.4" },
  { name: "fmt7", format: 7, size: 36, version: "1.4" },
  { name: "fmt8", format: 8, size: 38, version: "1.4" },
] as const;

const POINT_COUNT = 512;

beforeAll(async () => {
  await initLazCodec(readFileSync(fileURLToPath(WASM)));
});

describe("readLasHeader", () => {
  it.each(CASES)("reads the $name header", (c) => {
    const header = readLasHeader(fixture(`${c.name}.laz`));
    expect(header.version).toBe(c.version);
    expect(header.pointFormat).toBe(c.format);
    expect(header.pointSize).toBe(c.size);
    expect(header.pointCount).toBe(POINT_COUNT);
    expect(header.compressed).toBe(true);
    expect(header.vlrCount).toBe(1);
    expect(header.vlrsComplete).toBe(true);
    expect(Array.from(header.scale)).toEqual([0.001, 0.001, 0.001]);
    expect(Array.from(header.offset)).toEqual([0, 0, 0]);
    // The header stores the bounds interleaved as max then min per axis; a
    // reader that walks them in order comes back with min above max.
    for (let axis = 0; axis < 3; axis++) {
      expect(header.min[axis]!).toBeLessThan(header.max[axis]!);
    }
    header.free();
  });

  it("marks the uncompressed twin as uncompressed and VLR-free", () => {
    const header = readLasHeader(fixture("fmt3.las"));
    expect(header.compressed).toBe(false);
    expect(header.pointFormat).toBe(3);
    expect(header.vlrCount).toBe(0);
    expect(header.vlrsComplete).toBe(true);
    expect(header.offsetToPointData).toBe(header.headerSize);
    header.free();
  });

  it("finds the laszip VLR and hands back its payload", () => {
    const header = readLasHeader(fixture("fmt6.laz"));
    const vlr = header.findVlr(LASZIP_USER_ID, LASZIP_RECORD_ID);
    expect(vlr).toBeDefined();
    expect(vlr!.recordId).toBe(LASZIP_RECORD_ID);

    // The payload is what a chunk decoder is built from. This one describes the
    // file's own fixed-size chunks, so it differs from the variable-size VLR in
    // `fmt6.vlr` by exactly that field.
    const decoder = new LazChunkDecoder(vlr!.data);
    expect(decoder.pointSize).toBe(30);
    expect(decoder.chunkSize).toBe(128);
    decoder.free();
    vlr!.free();
    header.free();
  });

  it("reads a LAS 1.4 count from the 64-bit field, not the zeroed legacy one", () => {
    // Point formats 6 and up require the legacy u32 count to be 0, so a reader
    // that prefers it reports an empty file.
    const bytes = fixture("fmt6.laz");
    const legacy = new DataView(
      bytes.buffer,
      bytes.byteOffset,
      bytes.byteLength,
    ).getUint32(107, true);
    expect(legacy).toBe(0);
    const header = readLasHeader(bytes);
    expect(header.pointCount).toBe(POINT_COUNT);
    header.free();
  });

  it("reports an incomplete VLR directory instead of guessing", () => {
    const bytes = fixture("fmt6.laz");
    // Past the 375-byte header, short of the VLR that follows it.
    const header = readLasHeader(bytes.subarray(0, 380));
    expect(header.vlrCount).toBe(1);
    expect(header.vlrsComplete).toBe(false);
    expect(header.vlrs).toHaveLength(0);
    // The rest of the header is still trustworthy — that is the point of
    // reading a prefix at all.
    expect(header.pointCount).toBe(POINT_COUNT);
    expect(header.offsetToPointData).toBeGreaterThan(380);
    header.free();
  });

  it("rejects bytes that are not LAS at all", () => {
    expect(() => readLasHeader(new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]))).toThrow(
      /LASF/,
    );
  });

  it("rejects a header cut short", () => {
    expect(() => readLasHeader(fixture("fmt0.laz").subarray(0, 100))).toThrow(
      /truncated/i,
    );
  });
});

describe("decodeLazFile", () => {
  it.each(CASES)("decodes $name back to its uncompressed twin", (c) => {
    const decoded = decodeLazFile(fixture(`${c.name}.laz`));
    expect(decoded.pointCount).toBe(POINT_COUNT);
    expect(decoded.pointSize).toBe(c.size);
    expect(decoded.points).toEqual(lasPoints(c.name));
    decoded.free();
  });

  it("passes an uncompressed file through unchanged", () => {
    const decoded = decodeLazFile(fixture("fmt3.las"));
    expect(decoded.points).toEqual(lasPoints("fmt3"));
    decoded.free();
  });

  it("refuses a file whose point block was cut off", () => {
    const bytes = fixture("fmt0.las");
    expect(() => decodeLazFile(bytes.subarray(0, bytes.length - 100))).toThrow(
      /holds only/,
    );
  });

  it("reports a corrupt chunk rather than trapping", () => {
    const bytes = fixture("fmt0.laz").slice();
    // Past the header and the chunk-table offset, into the arithmetic stream.
    bytes.fill(0xff, 400, 600);
    expect(() => decodeLazFile(bytes)).toThrow(/failed to decode/);
    // The module is still usable: the failure was a Result, not a trap.
    const ok = decodeLazFile(fixture("fmt0.laz"));
    expect(ok.pointCount).toBe(POINT_COUNT);
    ok.free();
  });
});

describe("LazChunkDecoder", () => {
  it.each(CASES)("decodes a bare $name chunk", (c) => {
    const decoder = new LazChunkDecoder(fixture(`${c.name}.vlr`));
    expect(decoder.pointSize).toBe(c.size);
    // The fixtures use variable-size chunks, as COPC does.
    expect(decoder.chunkSize).toBe(0);
    expect(decoder.decode(fixture(`${c.name}.chunk`), POINT_COUNT)).toEqual(
      lasPoints(c.name),
    );
    decoder.free();
  });

  it("is reusable across nodes and order-independent", () => {
    const decoder = new LazChunkDecoder(fixture("fmt7.vlr"));
    const chunk = fixture("fmt7.chunk");
    const first = decoder.decode(chunk, POINT_COUNT);
    const second = decoder.decode(chunk, POINT_COUNT);
    // No cursor, no carry-over: the second call must not depend on the first.
    expect(second).toEqual(first);
    decoder.free();
  });

  it("decodes a prefix of a chunk when the caller asks for fewer points", () => {
    const decoder = new LazChunkDecoder(fixture("fmt2.vlr"));
    const head = decoder.decode(fixture("fmt2.chunk"), 100);
    expect(head).toEqual(lasPoints("fmt2").subarray(0, 100 * 26));
    decoder.free();
  });

  it("skips the fields the selection leaves out", () => {
    // Only the layered formats can skip work. XY, the return counts and the
    // scanner channel always come back; a field left out keeps the first
    // point's value, repeated, because laszip stores that point raw.
    const decoder = new LazChunkDecoder(fixture("fmt6.vlr"));
    const chunk = fixture("fmt6.chunk");
    const all = decoder.decode(chunk, POINT_COUNT);
    const some = decoder.decodeSelective(
      chunk,
      POINT_COUNT,
      LazField.XY_RETURNS_CHANNEL,
    );
    decoder.free();

    const view = (b: Uint8Array, at: number) =>
      new DataView(b.buffer, b.byteOffset, b.byteLength).getInt32(at, true);

    const firstZ = view(all, 8);
    const firstGps = all.subarray(22, 30);
    for (let i = 1; i < 9; i++) {
      const at = i * 30;
      expect(view(some, at)).toBe(view(all, at)); // X, always decompressed
      expect(view(some, at + 4)).toBe(view(all, at + 4)); // Y, likewise
      // Z really did move in this chunk, and the selective pass really did not
      // follow it.
      expect(view(all, at + 8)).not.toBe(firstZ);
      expect(view(some, at + 8)).toBe(firstZ);
      // GPS time occupies the last eight bytes of a format 6 record.
      expect(some.subarray(at + 22, at + 30)).toEqual(firstGps);
      expect(all.subarray(at + 22, at + 30)).not.toEqual(firstGps);
    }
  });

  it("refuses a laszip VLR payload that is not one", () => {
    expect(() => new LazChunkDecoder(new Uint8Array(8))).toThrow();
  });

  it("reports a corrupt chunk rather than trapping", () => {
    const decoder = new LazChunkDecoder(fixture("fmt3.vlr"));
    const bad = fixture("fmt3.chunk").slice();
    bad.fill(0xff, 64, 512);
    expect(() => decoder.decode(bad, POINT_COUNT)).toThrow(/failed to decode/);
    expect(decoder.decode(fixture("fmt3.chunk"), POINT_COUNT)).toEqual(
      lasPoints("fmt3"),
    );
    decoder.free();
  });

  it("refuses a point count that would allocate more than the ceiling", () => {
    const decoder = new LazChunkDecoder(fixture("fmt8.vlr"));
    // A corrupt hierarchy entry is just a number in a file; the ceiling is what
    // keeps it from becoming an allocation failure with no explanation.
    expect(() => decoder.decode(fixture("fmt8.chunk"), 0xffffffff)).toThrow(
      /ceiling/,
    );
    decoder.free();
  });
});

describe("decodeLazChunks", () => {
  it("decodes several nodes with one decoder", () => {
    const chunk = fixture("fmt2.chunk");
    const out = decodeLazChunks(fixture("fmt2.vlr"), [
      { data: chunk, pointCount: POINT_COUNT },
      { data: chunk, pointCount: 10 },
    ]);
    const expected = lasPoints("fmt2");
    expect(out[0]).toEqual(expected);
    expect(out[1]).toEqual(expected.subarray(0, 10 * 26));
  });
});
