//! LAZ decoding for voxelkloud, compiled to wasm.
//!
//! This crate is a codec and a frame reader, not a format driver. It answers
//! two questions — *where are the compressed points* and *what are the bytes
//! behind them* — and stops there. Which of those bytes is an intensity and
//! which is a classification is the driver's business, because the answer
//! differs between COPC, EPT and a bare `.laz`, while the arithmetic below does
//! not.
//!
//! Five callers share it: the COPC driver (COPC *is* laszip), the EPT driver
//! (3DEP on S3 ships `laszip` payloads), the single-file tier for a `.laz`
//! dropped on the page, the converter's input path, and the toolchain that
//! replaces PotreeConverter.
//!
//! Two decode shapes, because the callers genuinely differ:
//!
//! - [`LazChunkDecoder`] decompresses one laszip chunk standing on its own —
//!   no chunk table, no LAS header, point count supplied by the caller. This is
//!   the indexed path: COPC stores exactly one chunk per node, and EPT's
//!   hierarchy gives a point count per node. Build it once per file and reuse
//!   it across thousands of nodes; parsing the laszip VLR is the expensive part
//!   and it does not change between nodes.
//! - [`decode_laz_file`] takes a whole `.laz` and hands back its header and
//!   every point record. This is the unindexed path.
//!
//! Both return *raw LAS point records*, little-endian, exactly as an
//! uncompressed `.las` would store them.

// The LAS framing is shared with the native toolchain rather than kept twice.
// Two copies of a header parser is two sets of byte offsets, and the second one
// is always the one that is wrong.
use voxelkloud_io::las;

use std::io::Cursor;

use laz::las::selective::DecompressionSelection;
use laz::record::{
    LayeredPointRecordDecompressor, RecordDecompressor, SequentialPointRecordDecompressor,
};
use laz::{LasZipDecompressor, LazItem, LazVlr};
use wasm_bindgen::prelude::*;

/// A ceiling on what one call may allocate, in decompressed bytes.
///
/// wasm32 has 4 GiB of address space and far less in practice, and a point
/// count is a number in an untrusted file. Without this, a corrupt header asks
/// for a terabyte and the module traps on an allocation failure that says
/// nothing about why. 1 GiB is comfortably above the single-file tier's
/// ~10–30M points and far below the point where the request stops being real.
const MAX_DECODED_BYTES: u64 = 1 << 30;

fn err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

fn check_budget(point_count: u64, point_size: u64) -> Result<usize, JsError> {
    let bytes = point_count.saturating_mul(point_size);
    if bytes > MAX_DECODED_BYTES {
        return Err(JsError::new(&format!(
            "refusing to decode {point_count} points of {point_size} bytes: \
             {bytes} bytes is over the {MAX_DECODED_BYTES}-byte ceiling for one call. \
             Decode node by node instead of whole-file."
        )));
    }
    Ok(bytes as usize)
}

/// Build the record decompressor the laz items call for.
///
/// laz-rs keeps its own version of this private behind the file-shaped
/// `LasZipDecompressor`, which insists on a chunk table we do not have. The
/// dispatch is the whole of it: versions 1 and 2 are sequential, 3 and 4 are
/// layered — that is, LAS point formats 6 and up.
fn record_decompressor<'a, R>(
    items: &Vec<LazItem>,
    input: R,
) -> Result<Box<dyn RecordDecompressor<R> + Send + Sync + 'a>, JsError>
where
    R: std::io::Read + std::io::Seek + Send + Sync + 'a,
{
    let first = items
        .first()
        .ok_or_else(|| JsError::new("laszip VLR describes no items"))?;

    let mut decompressor: Box<dyn RecordDecompressor<R> + Send + Sync + 'a> = match first.version() {
        1 | 2 => Box::new(SequentialPointRecordDecompressor::new(input)),
        3 | 4 => Box::new(LayeredPointRecordDecompressor::new(input)),
        other => {
            return Err(JsError::new(&format!(
                "unsupported laszip item version {other} for item type {:?}",
                first.item_type()
            )))
        }
    };

    decompressor.set_fields_from(items).map_err(err)?;
    Ok(decompressor)
}

/// Which fields to spend time on, as a bitmask for
/// [`LazChunkDecoder::decode_selective`].
///
/// Only the layered decompressor (LAS point formats 6 and up) can skip work;
/// for the sequential formats the mask is accepted and ignored, because their
/// fields are interleaved in one arithmetic stream and there is nothing to
/// skip.
///
/// The record stride never changes, and an unselected field is not zeroed: its
/// bytes hold whatever the *first* point of the chunk had, repeated, because
/// laszip stores that point raw and the decompressor carries it forward.
/// Treat unselected bytes as undefined and do not read them.
#[wasm_bindgen]
pub struct LazField;

#[wasm_bindgen]
impl LazField {
    #[wasm_bindgen(getter, js_name = ALL)]
    pub fn all() -> u32 {
        DecompressionSelection::ALL
    }
    /// X, Y, return numbers and the scanner channel. Always decompressed.
    #[wasm_bindgen(getter, js_name = XY_RETURNS_CHANNEL)]
    pub fn xy_returns_channel() -> u32 {
        DecompressionSelection::XY_RETURNS_CHANNEL
    }
    #[wasm_bindgen(getter, js_name = Z)]
    pub fn z() -> u32 {
        DecompressionSelection::Z
    }
    #[wasm_bindgen(getter, js_name = CLASSIFICATION)]
    pub fn classification() -> u32 {
        DecompressionSelection::CLASSIFICATION
    }
    #[wasm_bindgen(getter, js_name = FLAGS)]
    pub fn flags() -> u32 {
        DecompressionSelection::FLAGS
    }
    #[wasm_bindgen(getter, js_name = INTENSITY)]
    pub fn intensity() -> u32 {
        DecompressionSelection::INTENSITY
    }
    #[wasm_bindgen(getter, js_name = SCAN_ANGLE)]
    pub fn scan_angle() -> u32 {
        DecompressionSelection::SCAN_ANGLE
    }
    #[wasm_bindgen(getter, js_name = USER_DATA)]
    pub fn user_data() -> u32 {
        DecompressionSelection::USER_DATA
    }
    #[wasm_bindgen(getter, js_name = POINT_SOURCE_ID)]
    pub fn point_source_id() -> u32 {
        DecompressionSelection::POINT_SOURCE_ID
    }
    #[wasm_bindgen(getter, js_name = GPS_TIME)]
    pub fn gps_time() -> u32 {
        DecompressionSelection::GPS_TIME
    }
    #[wasm_bindgen(getter, js_name = RGB)]
    pub fn rgb() -> u32 {
        DecompressionSelection::RGB
    }
    #[wasm_bindgen(getter, js_name = NIR)]
    pub fn nir() -> u32 {
        DecompressionSelection::NIR
    }
    #[wasm_bindgen(getter, js_name = WAVEPACKET)]
    pub fn wavepacket() -> u32 {
        DecompressionSelection::WAVEPACKET
    }
    #[wasm_bindgen(getter, js_name = EXTRA_BYTES)]
    pub fn extra_bytes() -> u32 {
        DecompressionSelection::ALL_EXTRA_BYTES
    }
}

/// One variable length record, header fields and payload.
#[wasm_bindgen]
pub struct Vlr {
    inner: las::Vlr,
}

#[wasm_bindgen]
impl Vlr {
    #[wasm_bindgen(getter, js_name = userId)]
    pub fn user_id(&self) -> String {
        self.inner.user_id.clone()
    }
    #[wasm_bindgen(getter, js_name = recordId)]
    pub fn record_id(&self) -> u16 {
        self.inner.record_id
    }
    #[wasm_bindgen(getter)]
    pub fn description(&self) -> String {
        self.inner.description.clone()
    }
    /// True for records from the EVLR directory at the end of the file.
    #[wasm_bindgen(getter)]
    pub fn extended(&self) -> bool {
        self.inner.extended
    }
    /// The payload. Copies out of wasm memory, so hold the result.
    #[wasm_bindgen(getter)]
    pub fn data(&self) -> Vec<u8> {
        self.inner.data.clone()
    }
}

/// The LAS public header block, plus the VLRs the buffer reached.
#[wasm_bindgen]
pub struct LasHeader {
    inner: las::LasHeader,
}

#[wasm_bindgen]
impl LasHeader {
    #[wasm_bindgen(getter)]
    pub fn version(&self) -> String {
        format!("{}.{}", self.inner.version_major, self.inner.version_minor)
    }
    #[wasm_bindgen(getter, js_name = headerSize)]
    pub fn header_size(&self) -> u16 {
        self.inner.header_size
    }
    #[wasm_bindgen(getter, js_name = offsetToPointData)]
    pub fn offset_to_point_data(&self) -> u32 {
        self.inner.offset_to_point_data
    }
    /// Format id with the laszip compression bit masked off: 0–10.
    #[wasm_bindgen(getter, js_name = pointFormat)]
    pub fn point_format(&self) -> u8 {
        self.inner.point_format
    }
    /// Size of one decompressed record, extra bytes included.
    #[wasm_bindgen(getter, js_name = pointSize)]
    pub fn point_size(&self) -> u16 {
        self.inner.point_size
    }
    /// Returned as an f64 rather than a bigint: every caller in this repo
    /// counts points in `number`, and LAS 1.4's 64-bit field does not reach
    /// 2^53 in any real file.
    #[wasm_bindgen(getter, js_name = pointCount)]
    pub fn point_count(&self) -> f64 {
        self.inner.point_count as f64
    }
    #[wasm_bindgen(getter)]
    pub fn compressed(&self) -> bool {
        self.inner.compressed
    }
    #[wasm_bindgen(getter)]
    pub fn scale(&self) -> Vec<f64> {
        self.inner.scale.to_vec()
    }
    #[wasm_bindgen(getter)]
    pub fn offset(&self) -> Vec<f64> {
        self.inner.offset.to_vec()
    }
    #[wasm_bindgen(getter)]
    pub fn min(&self) -> Vec<f64> {
        self.inner.min.to_vec()
    }
    #[wasm_bindgen(getter)]
    pub fn max(&self) -> Vec<f64> {
        self.inner.max.to_vec()
    }
    /// Absolute offset of the first EVLR, or 0 when there are none.
    #[wasm_bindgen(getter, js_name = evlrOffset)]
    pub fn evlr_offset(&self) -> f64 {
        self.inner.evlr_offset as f64
    }
    #[wasm_bindgen(getter, js_name = evlrCount)]
    pub fn evlr_count(&self) -> u32 {
        self.inner.evlr_count
    }
    /// How many VLRs the header claims, which may exceed what was read.
    #[wasm_bindgen(getter, js_name = vlrCount)]
    pub fn vlr_count(&self) -> u32 {
        self.inner.vlr_count
    }
    /// False when the buffer ended before the VLR directory did — read further
    /// into the file and call again.
    #[wasm_bindgen(getter, js_name = vlrsComplete)]
    pub fn vlrs_complete(&self) -> bool {
        self.inner.vlrs_complete
    }
    #[wasm_bindgen(getter)]
    pub fn vlrs(&self) -> Vec<Vlr> {
        self.inner
            .vlrs
            .iter()
            .map(|v| Vlr { inner: v.clone() })
            .collect()
    }
    /// The one VLR with this user id and record id, if the buffer reached it.
    #[wasm_bindgen(js_name = findVlr)]
    pub fn find_vlr(&self, user_id: &str, record_id: u16) -> Option<Vlr> {
        self.inner
            .vlrs
            .iter()
            .find(|v| v.is(user_id, record_id))
            .map(|v| Vlr { inner: v.clone() })
    }
}

/// Read the public header block and as much of the VLR directory as `bytes`
/// holds.
///
/// Safe to call on a prefix: a driver fetches the first few kilobytes of a
/// remote file, reads the layout, and checks
/// [`LasHeader::vlrs_complete`] before trusting the VLR list.
#[wasm_bindgen(js_name = readLasHeader)]
pub fn read_las_header(bytes: &[u8]) -> Result<LasHeader, JsError> {
    las::LasHeader::read(bytes)
        .map(|inner| LasHeader { inner })
        .map_err(err)
}

/// Read an EVLR directory from a buffer that begins *at the first record*.
///
/// That is the shape a ranged `GET` from `header.evlrOffset` produces, which is
/// how COPC's hierarchy is fetched. Records that did not fit are dropped
/// silently; compare the length against `count` to detect a short read.
#[wasm_bindgen(js_name = readLasEvlrs)]
pub fn read_las_evlrs(bytes: &[u8], count: u32) -> Vec<Vlr> {
    let (vlrs, _complete) = las::read_evlrs(bytes, count);
    vlrs.into_iter().map(|inner| Vlr { inner }).collect()
}

/// A decoder for laszip chunks that stand on their own.
///
/// Built once per file from the laszip VLR payload, then reused for every node.
/// It holds no cursor into the file and no state between calls, so nodes may be
/// decoded in any order, which is what an LOD scheduler does.
#[wasm_bindgen]
pub struct LazChunkDecoder {
    vlr: LazVlr,
}

#[wasm_bindgen]
impl LazChunkDecoder {
    /// Build from the payload of the `laszip encoded` VLR (record id 22204).
    ///
    /// That is `vlr.data`, not the 54-byte record header in front of it.
    #[wasm_bindgen(constructor)]
    pub fn new(laszip_vlr_record: &[u8]) -> Result<LazChunkDecoder, JsError> {
        let vlr = LazVlr::from_buffer(laszip_vlr_record).map_err(err)?;
        if vlr.items().is_empty() || vlr.items_size() == 0 {
            return Err(JsError::new(
                "laszip VLR describes a zero-byte point record, so a point count means nothing",
            ));
        }
        Ok(Self { vlr })
    }

    /// Size of one decompressed record, in bytes.
    #[wasm_bindgen(getter, js_name = pointSize)]
    pub fn point_size(&self) -> u32 {
        self.vlr.items_size() as u32
    }

    /// Points per chunk, or 0 when the file uses variable-size chunks — which
    /// COPC always does, one chunk per node.
    #[wasm_bindgen(getter, js_name = chunkSize)]
    pub fn chunk_size(&self) -> u32 {
        if self.vlr.uses_variable_size_chunks() {
            0
        } else {
            self.vlr.chunk_size()
        }
    }

    /// Decompress one chunk into `point_count` raw LAS point records.
    ///
    /// `compressed` is the chunk's bytes and nothing else — no chunk table, no
    /// leading offset. `point_count` comes from the index that pointed here:
    /// the COPC hierarchy entry, or the EPT node count.
    pub fn decode(&self, compressed: &[u8], point_count: u32) -> Result<Vec<u8>, JsError> {
        self.decode_selective(compressed, point_count, DecompressionSelection::ALL)
    }

    /// As [`Self::decode`], but only spends time on the fields in `selection`.
    ///
    /// See [`LazField`] for the bits and for what lands in the bytes of a field
    /// that was not selected — which is not zero.
    #[wasm_bindgen(js_name = decodeSelective)]
    pub fn decode_selective(
        &self,
        compressed: &[u8],
        point_count: u32,
        selection: u32,
    ) -> Result<Vec<u8>, JsError> {
        let point_size = self.vlr.items_size();
        let len = check_budget(point_count as u64, point_size)?;
        let mut out = vec![0u8; len];
        if len == 0 {
            return Ok(out);
        }

        let mut decompressor = record_decompressor(self.vlr.items(), Cursor::new(compressed))?;
        decompressor.set_selection(DecompressionSelection(selection));
        decompressor.decompress_many(&mut out).map_err(|e| {
            JsError::new(&format!(
                "failed to decode a {}-byte chunk of {point_count} points: {e}",
                compressed.len()
            ))
        })?;
        Ok(out)
    }
}

/// A whole file's header and its point records.
#[wasm_bindgen]
pub struct DecodedLas {
    header: las::LasHeader,
    points: Vec<u8>,
}

#[wasm_bindgen]
impl DecodedLas {
    #[wasm_bindgen(getter)]
    pub fn header(&self) -> LasHeader {
        LasHeader {
            inner: self.header.clone(),
        }
    }

    /// The raw LAS point records, `pointCount * pointSize` bytes.
    ///
    /// Copies out of wasm memory. Take it once and keep it — every read of this
    /// getter copies again.
    #[wasm_bindgen(getter)]
    pub fn points(&self) -> Vec<u8> {
        self.points.clone()
    }

    #[wasm_bindgen(getter, js_name = pointCount)]
    pub fn point_count(&self) -> f64 {
        self.header.point_count as f64
    }

    #[wasm_bindgen(getter, js_name = pointSize)]
    pub fn point_size(&self) -> u16 {
        self.header.point_size
    }
}

/// Decode a whole `.laz` — or pass a `.las` straight through.
///
/// For the unindexed tier: a file dropped on the page, an EPT node fetched
/// whole, the converter's input. Indexed formats should build a
/// [`LazChunkDecoder`] once and decode node by node instead; this function
/// holds the entire decompressed cloud in wasm memory at once, and
/// [`MAX_DECODED_BYTES`] is the point where it refuses.
#[wasm_bindgen(js_name = decodeLazFile)]
pub fn decode_laz_file(bytes: &[u8]) -> Result<DecodedLas, JsError> {
    let header = las::LasHeader::read(bytes).map_err(err)?;
    let point_size = header.point_size as u64;
    let len = check_budget(header.point_count, point_size)?;

    let start = header.offset_to_point_data as usize;
    if start > bytes.len() {
        return Err(JsError::new(&format!(
            "LAS header points at byte {start} for its point data, past the {} bytes given",
            bytes.len()
        )));
    }

    if !header.compressed {
        // Uncompressed input still goes through here so the single-file tier
        // has one entry point for `.las` and `.laz` alike.
        let end = start.saturating_add(len);
        if end > bytes.len() {
            return Err(JsError::new(&format!(
                "LAS file declares {} points of {point_size} bytes but holds only {} after \
                 its header",
                header.point_count,
                bytes.len() - start
            )));
        }
        return Ok(DecodedLas {
            header,
            points: bytes[start..end].to_vec(),
        });
    }

    if !header.vlrs_complete {
        return Err(JsError::new(
            "LAZ file is truncated before the end of its VLR directory",
        ));
    }
    let record = header
        .laszip_record()
        .ok_or_else(|| JsError::new("LAZ file has no laszip VLR (user id 'laszip encoded')"))?;
    let vlr = LazVlr::from_buffer(record).map_err(err)?;

    let mut points = vec![0u8; len];
    if len > 0 {
        // Seekable, so the decompressor finds the chunk table itself and
        // handles fixed and variable chunk sizes alike.
        let mut cursor = Cursor::new(bytes);
        cursor.set_position(start as u64);
        let mut decompressor = LasZipDecompressor::new(cursor, vlr).map_err(err)?;
        decompressor.decompress_many(&mut points).map_err(|e| {
            JsError::new(&format!(
                "failed to decode {} points from a {}-byte LAZ file: {e}",
                header.point_count,
                bytes.len()
            ))
        })?;
    }

    Ok(DecodedLas { header, points })
}
