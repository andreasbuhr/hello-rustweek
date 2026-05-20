#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

extern crate alloc;

use defmt_rtt as _;

use core::convert::Infallible;
use core::fmt::Write;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    mono_font::{ascii::FONT_8X13, MonoTextStyleBuilder},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::Rectangle,
    text::Text,
};
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    dma::DmaTxBuf,
    dma_tx_buffer,
    gpio::{Level, Output, OutputConfig},
    interrupt::software::SoftwareInterruptControl,
    lcd_cam::{lcd::i8080::{Config, I8080}, LcdCam},
    time::Rate,
    timer::timg::TimerGroup,
    Blocking,
};
use esp_radio::wifi::{AuthenticationMethod, scan::ScanConfig};
use heapless::String as HString;
use qrcode::{Color, QrCode};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    defmt::error!("panic: {}", defmt::Display2Format(info));
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

        (_, i8080, buf) = i8080.send(0x2C_u8, 0, buf).unwrap().wait();
        bytes_remaining -= buf.len();

        while bytes_remaining >= buf.len() {
            (_, i8080, buf) = i8080.send(0x3C_u8, 0, buf).unwrap().wait();
            bytes_remaining -= buf.len();
        }
        if bytes_remaining > 0 {
            buf.set_length(bytes_remaining);
            (_, i8080, buf) = i8080.send(0x3C_u8, 0, buf).unwrap().wait();
        }
        buf.set_length(buf.capacity());

        self.resources = Some((i8080, buf));
    }

    /// Draw a QR code centred horizontally, starting at `y_offset` from the top.
    fn draw_qr(&mut self, colors: &[Color], qr_modules: usize, module_size: usize, y_offset: u16) {
        let qr_pixels = qr_modules * module_size;
        let margin_x = (WIDTH as usize - qr_pixels) / 2;
        let x0 = COL_OFFSET + margin_x as u16;
        let y0 = y_offset;

        self.set_window(x0, y0, x0 + qr_pixels as u16 - 1, y0 + qr_pixels as u16 - 1);

        let (mut i8080, mut buf) = self.resources.take().unwrap();
        let row_bytes = qr_pixels * 2;
        let mut first = true;

        for qr_row in 0..qr_modules {
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

            for _ in 0..module_size {
                let cmd = if first { first = false; 0x2C_u8 } else { 0x3C_u8 };
                (_, i8080, buf) = i8080.send(cmd, 0, buf).unwrap().wait();
            }
        }

        buf.set_length(buf.capacity());
        self.resources = Some((i8080, buf));
    }
}

impl DrawTarget for Bus<'_> {
    type Color = Rgb565;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(Point { x, y }, color) in pixels {
            if x >= 0 && x < WIDTH as i32 && y >= 0 && y < HEIGHT as i32 {
                let px = x as u16 + COL_OFFSET;
                let py = y as u16;
                let rgb = color.into_storage();
                self.set_window(px, py, px, py);
                self.send(0x2C, &[(rgb >> 8) as u8, rgb as u8]);
            }
        }
        Ok(())
    }

    // Efficient batch path used by MonoTextStyle when background_color is set.
    fn fill_contiguous<I>(&mut self, area: &Rectangle, colors: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Self::Color>,
    {
        let display = Rectangle::new(Point::zero(), Size::new(WIDTH as u32, HEIGHT as u32));
        let clipped = area.intersection(&display);

        if clipped.size == Size::zero() {
            for _ in colors {}
            return Ok(());
        }

        if clipped == *area {
            let x0 = area.top_left.x as u16 + COL_OFFSET;
            let y0 = area.top_left.y as u16;
            let x1 = x0 + area.size.width as u16 - 1;
            let y1 = y0 + area.size.height as u16 - 1;
            self.set_window(x0, y0, x1, y1);

            let (mut i8080, mut buf) = self.resources.take().unwrap();
            let capacity = buf.capacity();
            let mut idx = 0usize;
            let mut first = true;

            for color in colors {
                let rgb = color.into_storage();
                {
                    let slice = buf.as_mut_slice();
                    slice[idx] = (rgb >> 8) as u8;
                    slice[idx + 1] = rgb as u8;
                }
                idx += 2;
                if idx >= capacity {
                    buf.set_length(capacity);
                    let cmd = if first { first = false; 0x2C_u8 } else { 0x3C_u8 };
                    (_, i8080, buf) = i8080.send(cmd, 0, buf).unwrap().wait();
                    idx = 0;
                }
            }
            if idx > 0 {
                buf.set_length(idx);
                let cmd = if first { 0x2C_u8 } else { 0x3C_u8 };
                (_, i8080, buf) = i8080.send(cmd, 0, buf).unwrap().wait();
            }
            buf.set_length(capacity);
            self.resources = Some((i8080, buf));
        } else {
            self.draw_iter(
                area.points()
                    .zip(colors)
                    .filter(|(pt, _)| clipped.contains(*pt))
                    .map(|(pt, color)| Pixel(pt, color)),
            )?;
        }

        Ok(())
    }
}

impl OriginDimensions for Bus<'_> {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

/// Build the WiFi QR-code URI per the meCard/WiFi standard.
/// Format: `WIFI:T:<type>;S:<ssid>;H:false;;`
fn wifi_uri(ssid: &str, auth: &Option<AuthenticationMethod>) -> HString<64> {
    let t = match auth {
        None | Some(AuthenticationMethod::None) => "nopass",
        Some(AuthenticationMethod::Wep) => "WEP",
        _ => "WPA",
    };
    let mut s: HString<64> = HString::new();
    write!(s, "WIFI:T:{t};S:{ssid};H:false;;").ok();
    s
}

fn auth_label(auth: &Option<AuthenticationMethod>) -> &'static str {
    match auth {
        None => "?",
        Some(AuthenticationMethod::None) => "Open",
        Some(AuthenticationMethod::Wep) => "WEP",
        Some(AuthenticationMethod::Wpa) => "WPA",
        Some(AuthenticationMethod::Wpa2Personal) | Some(AuthenticationMethod::WpaWpa2Personal) => {
            "WPA2"
        }
        Some(AuthenticationMethod::Wpa3Personal) | Some(AuthenticationMethod::Wpa2Wpa3Personal) => {
            "WPA3"
        }
        Some(_) => "Other",
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    esp_alloc::heap_allocator!(size: 72 * 1024);

    defmt::info!("hello-rustweek booting");

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_ints = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_ints.software_interrupt0);

    let delay = Delay::new();

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

    reset.set_low();
    delay.delay_millis(10);
    reset.set_high();
    delay.delay_millis(120);

    let mut bus = Bus::new(i8080, dma_buf);

    bus.send(0x01, &[]);     // SWRESET
    delay.delay_millis(150);
    bus.send(0x11, &[]);     // SLPOUT
    delay.delay_millis(10);
    bus.send(0x3A, &[0x55]); // COLMOD: RGB565
    bus.send(0x36, &[0x00]); // MADCTL: portrait, RGB order
    bus.send(0x21, &[]);     // INVON
    bus.send(0x13, &[]);     // NORON
    bus.send(0x29, &[]);     // DISPON
    delay.delay_millis(10);

    backlight.set_high();
    bus.fill_screen(WHITE);

    let (mut controller, _interfaces) =
        esp_radio::wifi::new(peripherals.WIFI, Default::default()).unwrap();

    let text_style = MonoTextStyleBuilder::new()
        .font(&FONT_8X13)
        .text_color(Rgb565::BLACK)
        .background_color(Rgb565::WHITE)
        .build();

    loop {
        let results = controller
            .scan_async(&ScanConfig::default())
            .await
            .unwrap_or_default();

        defmt::info!("scan complete: {} networks", results.len());

        if results.is_empty() {
            defmt::warn!("no networks found");
            bus.fill_screen(WHITE);
            Text::new("No networks found", Point::new(5, 20), text_style)
                .draw(&mut bus)
                .ok();
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }

        for ap in &results {
            defmt::info!(
                "AP ssid={} rssi={} ch={}",
                ap.ssid.as_str(),
                ap.signal_strength,
                ap.channel,
            );
            bus.fill_screen(WHITE);

            let ssid = ap.ssid.as_str();
            let uri = wifi_uri(ssid, &ap.auth_method);

            // QR code — scale to fit width, place at top with a small margin
            let qr_y: u16 = 5;
            if let Ok(code) = QrCode::new(uri.as_str().as_bytes()) {
                let qr_modules = code.width();
                let module_size = (WIDTH as usize / qr_modules).max(1);
                let colors = code.into_colors();
                bus.draw_qr(&colors, qr_modules, module_size, qr_y);

                let qr_pixels = (qr_modules * module_size) as u16;
                let text_base = qr_y + qr_pixels + 12;

                let mut line: HString<32> = HString::new();
                write!(line, "{ssid}").ok();
                Text::new(&line, Point::new(5, text_base as i32 + 13), text_style)
                    .draw(&mut bus).ok();

                line.clear();
                write!(line, "RSSI: {} dBm", ap.signal_strength).ok();
                Text::new(&line, Point::new(5, text_base as i32 + 31), text_style)
                    .draw(&mut bus).ok();

                line.clear();
                write!(line, "Ch: {}  Auth: {}", ap.channel, auth_label(&ap.auth_method)).ok();
                Text::new(&line, Point::new(5, text_base as i32 + 49), text_style)
                    .draw(&mut bus).ok();
            } else {
                // URI too long for QR — fall back to text only
                let mut line: HString<32> = HString::new();
                write!(line, "{ssid}").ok();
                Text::new(&line, Point::new(5, 20), text_style).draw(&mut bus).ok();

                line.clear();
                write!(line, "RSSI: {} dBm", ap.signal_strength).ok();
                Text::new(&line, Point::new(5, 40), text_style).draw(&mut bus).ok();

                line.clear();
                write!(line, "Ch: {}  Auth: {}", ap.channel, auth_label(&ap.auth_method)).ok();
                Text::new(&line, Point::new(5, 60), text_style).draw(&mut bus).ok();
            }

            Timer::after(Duration::from_secs(6)).await;
        }
    }
}
