#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::dma::DmaTxBuf;
use esp_hal::dma_tx_buffer;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::lcd_cam::lcd::i8080::{Config, I8080};
use esp_hal::lcd_cam::LcdCam;
use esp_hal::time::Rate;
use esp_hal::{Blocking, main};
use qrcode::{Color, QrCode};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

// T-Display-S3: ST7789V, 170x320 physical panel on a 240x320 GRAM
// Column offset = (240 - 170) / 2 = 35
const WIDTH: u16 = 170;
const HEIGHT: u16 = 320;
const COL_OFFSET: u16 = 35;

const WHITE: u16 = 0xFFFF;
const BLACK: u16 = 0x0000;

struct Bus<'d> {
    resources: Option<(I8080<'d, Blocking>, DmaTxBuf)>,
}

impl<'d> Bus<'d> {
    fn new(i8080: I8080<'d, Blocking>, buf: DmaTxBuf) -> Self {
        Self {
            resources: Some((i8080, buf)),
        }
    }

    fn send(&mut self, cmd: u8, data: &[u8]) {
        let (i8080, mut buf) = self.resources.take().unwrap();
        buf.fill(data);
        let (_, i8080, buf) = i8080.send(cmd, 0, buf).unwrap().wait();
        self.resources = Some((i8080, buf));
    }

    fn set_window(&mut self, x0: u16, y0: u16, x1: u16, y1: u16) {
        self.send(0x2A, &[
            (x0 >> 8) as u8, x0 as u8,
            (x1 >> 8) as u8, x1 as u8,
        ]);
        self.send(0x2B, &[
            (y0 >> 8) as u8, y0 as u8,
            (y1 >> 8) as u8, y1 as u8,
        ]);
    }

    fn fill_screen(&mut self, color: u16) {
        self.set_window(COL_OFFSET, 0, COL_OFFSET + WIDTH - 1, HEIGHT - 1);

        let (mut i8080, mut buf) = self.resources.take().unwrap();
        let color_bytes = color.to_be_bytes();
        for chunk in buf.as_mut_slice().chunks_mut(2) {
            chunk.copy_from_slice(&color_bytes);
        }
        buf.set_length(buf.capacity());

        let mut bytes_remaining = WIDTH as usize * HEIGHT as usize * 2;

        (_, i8080, buf) = i8080.send(0x2C_u8, 0, buf).unwrap().wait(); // RAMWR
        bytes_remaining -= buf.len();

        while bytes_remaining >= buf.len() {
            (_, i8080, buf) = i8080.send(0x3C_u8, 0, buf).unwrap().wait(); // RAMWRC
            bytes_remaining -= buf.len();
        }
        if bytes_remaining > 0 {
            buf.set_length(bytes_remaining);
            (_, i8080, buf) = i8080.send(0x3C_u8, 0, buf).unwrap().wait();
        }
        buf.set_length(buf.capacity());

        self.resources = Some((i8080, buf));
    }

    /// Render a QR code centered on the display.
    /// `colors` is the flat row-major slice from `QrCode::into_colors()`.
    fn draw_qr(&mut self, colors: &[Color], qr_modules: usize, module_size: usize) {
        let qr_pixels = qr_modules * module_size;

        // Center within the physical panel
        let margin_x = (WIDTH as usize - qr_pixels) / 2;
        let margin_y = (HEIGHT as usize - qr_pixels) / 2;
        let x0 = COL_OFFSET + margin_x as u16;
        let y0 = margin_y as u16;

        self.set_window(x0, y0, x0 + qr_pixels as u16 - 1, y0 + qr_pixels as u16 - 1);

        let (mut i8080, mut buf) = self.resources.take().unwrap();
        let row_bytes = qr_pixels * 2;
        let mut first = true;

        for qr_row in 0..qr_modules {
            // Build one scaled display row in the DMA buffer
            {
                let slice = buf.as_mut_slice();
                let mut idx = 0;
                for qr_col in 0..qr_modules {
                    let pixel = colors[qr_row * qr_modules + qr_col].select(BLACK, WHITE);
                    let [hi, lo] = pixel.to_be_bytes();
                    for _ in 0..module_size {
                        slice[idx] = hi;
                        slice[idx + 1] = lo;
                        idx += 2;
                    }
                }
            }
            buf.set_length(row_bytes);

            // Repeat each module row vertically
            for _ in 0..module_size {
                let cmd = if first { first = false; 0x2C_u8 } else { 0x3C_u8 };
                (_, i8080, buf) = i8080.send(cmd, 0, buf).unwrap().wait();
            }
        }

        buf.set_length(buf.capacity());
        self.resources = Some((i8080, buf));
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[main]
fn main() -> ! {
    esp_alloc::heap_allocator!(size: 32 * 1024);

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let delay = Delay::new();

    // Keep RD inactive (high) — we only write to the display
    let _rd = Output::new(peripherals.GPIO9, Level::High, OutputConfig::default());
    let mut backlight = Output::new(peripherals.GPIO38, Level::Low, OutputConfig::default());
    let mut reset = Output::new(peripherals.GPIO5, Level::High, OutputConfig::default());

    let dma_buf = dma_tx_buffer!(4000).unwrap();

    let lcd_cam = LcdCam::new(peripherals.LCD_CAM);
    let i8080 = I8080::new(
        lcd_cam.lcd,
        peripherals.DMA_CH0,
        Config::default().with_frequency(Rate::from_mhz(20)),
    )
    .unwrap()
    .with_cs(peripherals.GPIO6)
    .with_dc(peripherals.GPIO7)
    .with_wrx(peripherals.GPIO8)
    .with_data0(peripherals.GPIO39)
    .with_data1(peripherals.GPIO40)
    .with_data2(peripherals.GPIO41)
    .with_data3(peripherals.GPIO42)
    .with_data4(peripherals.GPIO45)
    .with_data5(peripherals.GPIO46)
    .with_data6(peripherals.GPIO47)
    .with_data7(peripherals.GPIO48);

    // Hardware reset
    reset.set_low();
    delay.delay_millis(10);
    reset.set_high();
    delay.delay_millis(120);

    let mut bus = Bus::new(i8080, dma_buf);

    // ST7789V init
    bus.send(0x01, &[]);     // SWRESET
    delay.delay_millis(150);
    bus.send(0x11, &[]);     // SLPOUT
    delay.delay_millis(10);
    bus.send(0x3A, &[0x55]); // COLMOD: RGB565
    bus.send(0x36, &[0x00]); // MADCTL: portrait, RGB order
    bus.send(0x21, &[]);     // INVON: inversion on (required for correct colors)
    bus.send(0x13, &[]);     // NORON
    bus.send(0x29, &[]);     // DISPON
    delay.delay_millis(10);

    backlight.set_high();

    // Generate QR code
    let code = QrCode::new(b"https://github.com/andreasbuhr/hello-rustweek").unwrap();
    let qr_modules = code.width();
    // Largest integer scale that fits in the narrower display dimension
    let module_size = (WIDTH as usize / qr_modules).max(1);
    let colors = code.into_colors();

    bus.fill_screen(WHITE);
    bus.draw_qr(&colors, qr_modules, module_size);

    loop {
        delay.delay_millis(1_000);
    }
}
