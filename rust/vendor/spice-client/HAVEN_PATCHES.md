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
- **GLZ_RGB decoder** (`decode_glz` + `glz_window` + `glz_decode_body`,
  `src/channels/display.rs`): port of the spice-gtk GLZ decoder. GLZ is LZSS whose
  back-references may reach into **earlier decoded images** held in a per-channel
  window (`glz_window: HashMap<image_id, GlzImage>`); each decoded image is inserted
  and the window is trimmed by id. Same 28-byte big-endian stream header as LZ.
- **GLZ enablement = `SpiceMsgcDisplayInit`, not link caps** (`src/protocol.rs`,
  `src/channels/display.rs`): the spice-server only emits GLZ_RGB once the client
  sends a `DISPLAY_INIT` with a **non-zero GLZ dictionary window**. Two bugs blocked
  this: `SpiceMsgcDisplayInit` was mis-padded (`pad_before=7` → 17 bytes, every field
  shifted) **and** lacked the `glz_dictionary_window_size` field entirely, so the
  server always saw a zero window and fell back to LZ_RGB. Fixed: packed layout
  (14 bytes) + `glz_dictionary_window_size: i32` set to 16 MiB−1 at all three
  DISPLAY_INIT sites. Caps/MINI_HEADER are **not** required (a 0-cap client gets GLZ).
- **`SpiceLinkMess` packed** (`src/protocol.rs`): removed `pad_before=2` (20→18
  bytes, `caps_offset=18`). Tolerated at 0 caps but corrupts `num_channel_caps` the
  moment any cap is advertised (server read 65536 → INVALID_DATA). Correctness fix.
  - Verified: a `image-compression=glz`-forced Windows Server 2025 streams ~115
    GLZ_RGB images per LogonUI paint; all decode with **0 warnings** and the captured
    frame (login window, glyphs, password dots, cursor) is pixel-correct.
- **ZLIB_GLZ_RGB decoder** (`decode_image_at` + `zlib_inflate`,
  `src/channels/display.rs`): `SpiceZlibGlzRGBData` (packed) = `glz_data_size u32` +
  `data_size u32` + zlib bytes. Inflate (flate2) then feed the result to `decode_glz`.
  Verified: a server with `<zlib compression='always'/>` (qemu
  `zlib-glz-wan-compression=always`) streams type-107 images mixed with GLZ — 80
  ZLIB_GLZ decoded with **0 warnings**, inflated size matched `glz_data_size` exactly,
  frame pixel-correct.
- **Pixmap cache (FROM_CACHE) decode + CACHE_ME store** (`decode_image_at`,
  `src/protocol.rs` flag consts): `DISPLAY_INIT` now enables the pixmap cache
  (`cache_id=1`, `cache_size=32Mi`, spice-gtk defaults). `decode_image_at` reads the
  descriptor `id`+`flags`, stores any decoded image flagged `SPICE_IMAGE_FLAGS_CACHE_ME`
  in `image_cache` (keyed by id), and resolves `FROM_CACHE` (103) / `FROM_CACHE_LOSSLESS`
  (106) by id (their wire body is empty). Server eviction (`INVAL_LIST`/`INVAL_ALL`) was
  already handled. The `ImageCache` get/insert/remove is unit-tested.
  - **NOT validated on real traffic.** The spice-server image cache is populated only
    from guest QXL **2D drawing commands**, which modern Windows does not emit: its QXL
    driver is a Display-**Only** Driver (KMDOD) — the framebuffer is *scraped* by the
    server, not driven by 2D ops. Verified empirically: across glz/zlib-glz/off and
    LogonUI/desktop/Explorer (~2100 images), incl. with the Red Hat QXL driver installed
    (`Win32_VideoController` = "Red Hat QXL controller"), the server set `CACHE_ME`
    **zero** times and never sent FROM_CACHE. Real FROM_CACHE needs a 2D-accel QXL guest
    (Win7/XPDM or old Linux xf86-video-qxl). The decode path is correct by construction
    but should be re-checked against such a guest if one becomes available.

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

## RESOLVED: the "streaming gap" was a client-side fatal MARK parse
Earlier this client appeared to receive only the **initial paint** and no
incremental updates. Root cause (found by a logging proxy + `Recv-Q` capture
against a default `auto_glz` QEMU): the `MSG_DISPLAY_MARK` (type 102) arrives
with an **empty (0-byte) body**, but `SpiceMsgDisplayMark::read` required bytes →
returned `Err` → `?` propagated out of `handle_message` **and out of `run()`**.
The display task is `tokio::spawn`ed and its `JoinHandle` error is never checked,
so the task **died silently** while the socket stayed open (held by `inner`); all
later display bytes then piled up unread (`Recv-Q` observed climbing to ~80 KB
during a screen change). The earlier "Recv-Q stays 0 / server isn't sending"
reading was wrong — the server *was* streaming. Two fixes:
- **MARK tolerates an empty/short body** (`display.rs`): parse the `u32` mark only
  when ≥4 bytes are present, else mark=0. Never fatal.
- **`run()` makes per-message handling non-fatal** (`display.rs`): a single
  message's parse/decode error is now `warn!`-logged and skipped instead of
  tearing down the whole channel. (read/IO errors still end the loop.)
- **DRAW_COPY clip RECTS is inline, not a pointer** (`display.rs`): a `RECTS` clip
  is `num_rects u32` + `num_rects * SpiceRect(16)` stored inline between
  `clip.type` and the `src_bitmap` @ptr32 — not a 4-byte pointer. The old code
  skipped 4 bytes → read `src_image=0` → decoded garbage for every clipped
  DRAW_COPY (e.g. scroll repaints). Verified by wire capture (`src_bitmap` offset
  = 21 + 4 + num_rects*16).

Verified end-to-end against default `auto_glz` QEMU (Ubuntu installer): a QMP
wheel-scroll of the language list streams incremental DRAW_COPY repaints that
decode pixel-correct (3 frames, `Recv-Q` stays 0, no decode warnings). NB the
repaints arrive as **LZ_RGB (101)**, so this validates the LZ path on incremental
frames. **GLZ (102) is now validated** on real `image-compression=glz` traffic — see
the GLZ enablement entry under "Applied" (Windows Server 2025, ~115 GLZ images/paint,
0 decode warnings, frame pixel-correct).

## TODO (in progress)
- QUIC (1) / LZ4 (109) image decoders.
- FROM_CACHE (103) real-traffic check against a 2D-accel QXL guest (decode implemented;
  modern Windows QXL is display-only so no cache hints — see the FROM_CACHE entry above).
- LZ_RGB16 / LZ_PLT sub-types (only RGB24/RGB32/RGBA decoded so far).
- Cursor channel shapes; multi-surface; remaining draw ops (FILL/OPAQUE/COPY_BITS).
- `MSG_PING` (type 4, 12-byte body) currently logged-and-skipped; add PONG if a
  server starts disconnecting idle clients.

Upstream these once stabilised.
