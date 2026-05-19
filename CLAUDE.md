# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Bare-metal Rust firmware for an **ESP32-S3** microcontroller (LilyGo **T-Display-S3** board) using `esp-hal`. This is a `no_std` project targeting the `xtensa-esp32s3-none-elf` triple. It uses async/await via `embassy` (wrapped by `esp-rtos`), has a 72 KB heap via `esp-alloc`, and drives an ST7789V display over a parallel 8-bit I8080 interface.

Current functionality: continuously scans Wi-Fi networks and displays each one on the LCD with a QR code (WiFi meCard URI) and network metadata (SSID, RSSI, channel, auth type).

## Toolchain

Requires Espressif's Rust fork (Xtensa support). `rust-toolchain.toml` pins `channel = "esp"`. Install via [espup](https://github.com/esp-rs/espup):

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

`cargo run` invokes `espflash flash --monitor --chip esp32s3` (configured in `.cargo/config.toml`).

## Architecture

**Entry point** (`src/bin/main.rs`): decorated with `#[esp_rtos::main]` and typed `async fn main(_spawner: Spawner) -> !`. This macro starts the embassy executor via `esp_rtos::start(...)` and drives async tasks. The first call in `main` must be `esp_alloc::heap_allocator!(size: 72 * 1024)` to set up the heap, then `esp_hal::init(config)` to initialize the HAL and claim the `Peripherals` singleton.

**Display driver** (`Bus<'d>` struct): wraps `I8080<'d, Blocking>` (parallel 8-bit LCD interface) and a `DmaTxBuf`. Implements `embedded_graphics::DrawTarget<Color = Rgb565>` so standard `embedded-graphics` primitives work directly. Key display constants: `WIDTH=170`, `HEIGHT=320`, `COL_OFFSET=35` (the ST7789V GRAM is 240 wide; the physical panel is centered).

**`src/lib.rs`**: currently just `#![no_std]`; shared code can go here.

**`build.rs`**: links `linkall.x` and emits friendly linker diagnostics for common missing-symbol errors.

## Display Hardware

- **Panel**: ST7789V, 170×320 physical pixels on a 240×320 GRAM (column offset 35)
- **Interface**: 8-bit parallel I8080 via `esp_hal::lcd_cam::lcd::i8080::I8080` at 20 MHz
- **Backlight**: GPIO38 (active high), **Reset**: GPIO5, **RD**: GPIO9 (held high)
- **Data bus**: GPIO39–GPIO42, GPIO45–GPIO48; **CS**: GPIO6; **DC**: GPIO7; **WR**: GPIO8
- **Init sequence**: SWRESET → SLPOUT → COLMOD(RGB565) → MADCTL(portrait) → INVON → NORON → DISPON

## Key Constraints

**`mem::forget` is denied** (`#![deny(clippy::mem_forget)]`). Many `esp-hal` types hold DMA buffers that must be returned to hardware on drop; forgetting them causes undefined behavior.

**Stack frame limit is 1024 bytes** (`.clippy.toml` + `#![deny(clippy::large_stack_frames)]` in `main.rs`). Allocate large buffers as `static` or on the heap, not on the stack. `main` itself has `#[allow(clippy::large_stack_frames)]` because top-level setup is exempt.

**Both `dev` and `release` profiles** use `opt-level = "s"` (size) because debug builds are otherwise too large for the device.

**`alloc` is available**: the crate is declared with `extern crate alloc` and the heap is initialized at startup, so `alloc` types (`Vec`, `String`, `Box`) can be used.

## Adding Features

- **Logging over RTT**: add `defmt` + `defmt-rtt` + `-Tdefmt.x` to `rustflags` in `.cargo/config.toml`.
- **Additional async tasks**: use the `Spawner` passed to `main` and `#[embassy_executor::task]`.
- **Examples reference**: `https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.1.0/examples`
