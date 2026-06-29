# spice-kotlin

Kotlin/Android bindings for a [SPICE](https://www.spice-space.org/) remote-desktop
client, exposed over [UniFFI](https://mozilla.github.io/uniffi-rs/). Built for
[Haven](https://github.com/GlassHaven/Haven) but usable standalone: it produces a
`libspice_transport.so` plus generated Kotlin (`sh.haven.spice`) presenting a small
synchronous client — frame callback (RGBA), cursor callback, lifecycle, and input
forwarding (keyboard, mouse motion/buttons/wheel).

## Layout

- `rust/` — the UniFFI wrapper (`src/lib.rs`) over the SPICE client, plus a host
  CLI (`src/bin/spice-cli.rs`) for pixel-level verification against a real server.
- `rust/vendor/spice-client/` — a **vendored fork** of `spice-client` 0.2.0
  (see Licensing). Its decoder is patched to render correctly against real
  QEMU/SPICE servers; every change is logged in
  [`rust/vendor/spice-client/HAVEN_PATCHES.md`](rust/vendor/spice-client/HAVEN_PATCHES.md).
- `kotlin/` — UniFFI-generated bindings (committed so consumers need no Rust toolchain).
- `jniLibs/{arm64-v8a,x86_64}/` — prebuilt `.so` (committed; CI consumes them directly).

## Build

```sh
tools/build-android.sh        # cargo-ndk → jniLibs + regenerated Kotlin bindings
```

Requires `rustup`, `cargo-ndk`, the `aarch64-linux-android`/`x86_64-linux-android`
targets, and an Android NDK (`ANDROID_NDK_HOME`). The Gradle `buildSpiceNative`
task wraps the same `cargo ndk` invocation. The committed `.so`/bindings mean a
plain Gradle consumer builds without the Rust toolchain.

## Licensing

This is a combined work under two compatible copyleft licenses:

- **The wrapper** (`rust/src/`, `kotlin/`, `build.gradle.kts`, `tools/`) is
  licensed **AGPL-3.0-or-later** — see [`LICENSE`](LICENSE).
- **The vendored `spice-client`** (`rust/vendor/spice-client/`) is **GPL-3.0**,
  a fork of [`arsfeld/quickemu-manager`](https://github.com/arsfeld/quickemu-manager);
  it retains its own [`LICENSE`](rust/vendor/spice-client/LICENSE) and the patch
  log credits the upstream.

AGPL-3.0 is compatible with GPL-3.0, so the combined binary is effectively
AGPL-3.0-or-later: if you run a modified version as a network service, the AGPL's
§13 source-offer obligation applies.
