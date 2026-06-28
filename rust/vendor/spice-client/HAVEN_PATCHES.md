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

- **GLZ_RGB decoder** (`decode_glz` + free `glz_decode_body`, `display.rs`): port
  of spice-gtk `decode-glz.c` / `decode-glz-tmpl.c`. GLZ is LZSS over a global
  dictionary window of previously-decoded images. 33-byte big-endian header
  (`[type|top_down]` packed byte, `id u64`, `win_head_dist u32`); references with
  `image_dist == 0` are same-image, else resolve against `glz_window[id-dist]`
  (kept on `DisplayChannel`, released past `win_head_dist`). RGB24/RGB32 + RGBA
  alpha pass; RGB16/PLT deferred. Unit-tested against hand-derived-from-reference
  control vectors (`glz_literal_then_same_image_run`, `glz_cross_image_reference`,
  `glz_truncated_returns_none`). **NOT yet validated against real QEMU GLZ
  traffic** — see the streaming-gap note below.
- **SPICE ACK flow control** (`display.rs`): the channel sent only `ACK_SYNC` on
  `SET_ACK`, never the periodic `MSGC_ACK`. Now parses the `window` and acks every
  `window` messages (necessary, though not sufficient — see below).
- **`SpiceRect` test** (`protocol/tests.rs`): updated to assert the corrected
  `{top,left,bottom,right}` wire order.

## KNOWN BLOCKER: no incremental display updates (streaming gap)
Empirically, this client receives only the **initial paint** (SET_ACK, INVAL,
SURFACE_CREATE, one DRAW_COPY, MARK) and then **no further display messages** —
confirmed across an off/lz/auto_glz QEMU and a live Windows desktop, with the
client socket `Recv-Q` staying 0 (the server is not sending more). So the gap is
client-side: something a real client (spice-gtk) sends to keep the server
streaming is missing. ACK windowing and per-message ACK were both tried and did
NOT unblock it. This blocks live validation of GLZ/QUIC/cache (they only exercise
on incremental updates) AND would make the on-device client show only the first
frame. Needs diffing against real spice-gtk wire traffic to find the missing
client message(s). The crate's "transport works" only ever covered frame 1.

## TODO (in progress)
- **Resolve the streaming gap above (highest priority — gates everything).**
- ZLIB_GLZ_RGB (107) / QUIC (1) / LZ4 (109) image decoders.
- LZ_RGB16 / LZ_PLT sub-types (only RGB24/RGB32/RGBA decoded so far).
- Cursor channel shapes; multi-surface; remaining draw ops (FILL/OPAQUE/COPY_BITS).

Upstream these once stabilised.
