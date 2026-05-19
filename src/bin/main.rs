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
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[main]
fn main() -> ! {
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
    bus.send(0x01, &[]);       // SWRESET
    delay.delay_millis(150);
    bus.send(0x11, &[]);       // SLPOUT — exit sleep
    delay.delay_millis(10);
    bus.send(0x3A, &[0x55]);   // COLMOD — RGB565
    bus.send(0x36, &[0x00]);   // MADCTL — portrait, RGB order
    bus.send(0x21, &[]);       // INVON — inversion on (required for correct colors)
    bus.send(0x13, &[]);       // NORON — normal display mode
    bus.send(0x29, &[]);       // DISPON — display on
    delay.delay_millis(10);

    backlight.set_high();

    // RGB565: R=0xF800, G=0x07E0, B=0x001F
    const COLORS: [u16; 3] = [0xF800, 0x07E0, 0x001F];

    let col_end = COL_OFFSET + WIDTH - 1;
    let row_end = HEIGHT - 1;

    let mut color_index = 0usize;
    loop {
        let color = COLORS[color_index % COLORS.len()];
        color_index = color_index.wrapping_add(1);

        // Set the write window to the full panel
        bus.send(0x2A, &[
            (COL_OFFSET >> 8) as u8, COL_OFFSET as u8,
            (col_end >> 8) as u8,    col_end as u8,
        ]);
        bus.send(0x2B, &[
            0x00, 0x00,
            (row_end >> 8) as u8, row_end as u8,
        ]);

        // Stream pixel data via DMA — 170*320*2 = 108 800 bytes in 4000-byte chunks
        let (mut i8080, mut buf) = bus.resources.take().unwrap();
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
            buf.set_length(buf.capacity());
        }

        bus.resources = Some((i8080, buf));

        delay.delay_millis(1_000);
    }
}
