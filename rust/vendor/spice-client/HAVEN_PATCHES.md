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
- LZ (100) / GLZ (101, QEMU `auto_glz` default) / ZLIB_GLZ (106) / QUIC (1) / LZ4 (108) image decoders.
- Cursor channel shapes; multi-surface; remaining draw ops (FILL/OPAQUE/COPY_BITS).

Upstream these once stabilised.
