# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Bare-metal Rust firmware for an **ESP32-S3** microcontroller using `esp-hal`. This is a `no_std`/`no_main` project targeting the `xtensa-esp32s3-none-elf` triple. No operating system, no heap by default.

## Toolchain

The project requires Espressif's Rust fork (Xtensa support). The `rust-toolchain.toml` pins `channel = "esp"`. Install via [espup](https://github.com/esp-rs/espup):

```sh
cargo install espup && espup install
```

Flashing requires `espflash`:

```sh
cargo install espflash
```

## Commands

```sh
# Build
cargo build
cargo build --release

# Flash and open serial monitor (device must be connected)
cargo run
cargo run --release

# Check without building the final binary
cargo check

# Lint
cargo clippy
```

`cargo run` invokes `espflash flash --monitor --chip esp32s3` (configured in `.cargo/config.toml` as the runner).

## Architecture

- `src/bin/main.rs` — entry point, decorated with `#[esp_hal::main]`. Must return `!`. Initializes HAL with `esp_hal::init(config)` which consumes a `Config` and gives back a `Peripherals` struct. Peripherals are accessed by destructuring that struct — they are move-only singletons.
- `src/lib.rs` — currently just `#![no_std]`; shared code goes here.
- `build.rs` — links `linkall.x` and registers itself as an error-handling script for the linker to emit friendly diagnostics on common undefined-symbol errors (e.g., missing `defmt`, missing `esp-alloc`).

## Key Constraints

**`mem::forget` is denied** (`#![deny(clippy::mem_forget)]`). Many `esp-hal` types hold DMA buffers that must be returned to hardware on drop; forgetting them causes undefined behavior.

**Stack frame limit is 1024 bytes** (`.clippy.toml`). Allocate large buffers as `static` or on the heap (requires adding `esp-alloc`), not on the stack.

**`no_std` / no heap**: `alloc` types (`Vec`, `String`, `Box`) are unavailable unless `esp-alloc` is added as a dependency and initialized at runtime.

**Both `dev` and `release` profiles** use `opt-level = "s"` (size) because debug builds are otherwise too large for the device.

## Adding Features

- **Heap**: add `esp-alloc` dependency and call `esp_alloc::heap_allocator!(size: N)` early in `main`.
- **Logging over RTT**: add `defmt` + `defmt-rtt` + add `-Tdefmt.x` to `rustflags` in `.cargo/config.toml`.
- **Networking / Wi-Fi / BLE**: add `esp-radio` (requires a scheduler: `esp-rtos` or similar).
- **Examples reference**: `https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.1.0/examples`
