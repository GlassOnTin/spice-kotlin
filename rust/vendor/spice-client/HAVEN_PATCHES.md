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

## TODO (in progress)
- LZ (100) / GLZ (101, QEMU `auto_glz` default) / ZLIB_GLZ (106) / QUIC (1) / LZ4 (108) image decoders.
- Cursor channel shapes; multi-surface; neutralise the fake DrawFill test pattern.

Upstream these once stabilised.
