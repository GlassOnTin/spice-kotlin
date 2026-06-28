# Haven patches to spice-client 0.2.0

Vendored fork of `spice-client` 0.2.0 (github.com/arsfeld/quickemu-manager, GPL-3.0)
with fixes to its display decoder, which does not render against real QEMU/SPICE
servers as published. Verified empirically against `qemu-system-x86_64 -vga qxl -spice`.

## Applied
- **`SpiceRect` wire order** (`src/protocol.rs`): was `{left,top,right,bottom}`,
  corrected to SPICE wire order `{top,left,bottom,right}`.
- **DRAW_COPY parse** (`src/channels/display.rs`): replaced the binrw
  `SpiceDrawCopy::read` path with an explicit wire parse. SPICE wire pointers are
  32-bit offsets (`@ptr32`), the crate modelled them as `u64`; `SpiceClip` is
  variable-length (1 byte for `NONE`), the crate read a fixed 12 bytes. Both
  misaligned `src_image`/`src_area`.
- **Image decode** (`decode_image_at` / `decode_bitmap_inline`): replaced the
  upstream `decode_image`, which mis-parsed `SpiceImageDescriptor` (spurious
  padding) and fabricated placeholder pixels (checkerboard / gray 32x32) on a
  bogus "cached address > 0x10000000" heuristic. Now decodes the real inline
  `SPICE_IMAGE_TYPE_BITMAP` (32BIT BGRx / 24BIT / RGBA) into RGBA.
- **De-fake / dead-code removal** (`src/channels/display.rs`): deleted the
  fabricating `decode_image` and the now-orphaned old `decode_bitmap` (both
  superseded by `decode_image_at`/`decode_bitmap_inline`, no live callers).
  `DRAW_FILL` and the unimplemented image codecs now `warn!`/`debug!` and leave
  the surface untouched — they never invent pixels. Verified: the `off`-server
  (raw BITMAP) still renders a correct 1024×768 Ubuntu installer frame.
- **Image-type constants** (`src/protocol.rs`): upstream had `SPICE_IMAGE_TYPE_*`
  shifted by one from 100 up (LZ=100, GLZ=101, FROM_CACHE=102, …), so every
  compressed image mis-dispatched. Corrected to the canonical spice-protocol
  `spice/enums.h` order: `LZ_PLT=100, LZ_RGB=101, GLZ_RGB=102, FROM_CACHE=103,
  SURFACE=104, JPEG=105, FROM_CACHE_LOSSLESS=106, ZLIB_GLZ_RGB=107,
  JPEG_ALPHA=108, LZ4=109`.
- **LZ_RGB decoder** (`decode_lz` + `lz_decompress`, `src/channels/display.rs`):
  port of spice-common `lz.c` / `lz_decompress_tmpl.c`. Wire layout (verified by
  byte capture against QEMU): ImageDescriptor(18) + `BinaryData{data_size u32 LE}`
  + LZ stream; the stream is a 28-byte **big-endian** header
  (magic `0x20205a4c`, version, type, width, height, stride, top_down) then the
  LZSS body (`MAX_COPY=32`, `MAX_DISTANCE=8191`). Handles RGB24/RGB32 (3 stream
  bytes/pixel → BGRX) and RGBA (extra alpha LZSS pass); RGB16/PLT deferred.
  Out-of-range back-references return `None` (surface left untouched, no panic).
  Verified: `image-compression=lz` server renders a correct 1024×768 frame.

## Design note: binrw structs vs. manual parse
The image/draw wire structs in `protocol.rs` (`SpiceImage`, `SpiceImageDescriptor`,
`SpiceBitmap`, `SpiceClip`, `SpiceDrawCopy`, `SpiceAddress`) still carry the
upstream's wrong layout (`SpiceAddress = u64`, spurious paddings, fixed-size
`SpiceClip`). This is deliberate: nothing in the live render path parses them via
binrw anymore — DRAW_COPY and image decode use the explicit manual parse above,
and `resolve_address` is dead. Those structs survive only as type annotations and
in `#[cfg(test)]` fixtures, so rewriting their layout would be busywork. The
structs that ARE read live (`SpiceMsgSurfaceCreate/Destroy`, `SpiceMonitorsConfig`,
`SpiceMsgDisplayMark`, `SpiceCopyBits`) are validated when their phases land
(F = COPY_BITS, H = surfaces).

## TODO (in progress)
- GLZ_RGB (102, QEMU `auto_glz` default) / ZLIB_GLZ_RGB (107) / QUIC (1) / LZ4 (109) image decoders.
- LZ_RGB16 / LZ_PLT sub-types (only RGB24/RGB32/RGBA decoded so far).
- Cursor channel shapes; multi-surface; remaining draw ops (FILL/OPAQUE/COPY_BITS).

Upstream these once stabilised.
