//! ESP32-S3 Doom-like lab — GPIO buttons + ILI9341 over SPI2.
//!
//! Build: `cargo build --release --features hw`
//! (package default target is `xtensa-esp32s3-none-elf` via `.cargo/config.toml`)

#![no_std]
#![no_main]
#![allow(static_mut_refs)]

use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    main,
    spi::master::{Config as SpiConfig, Spi},
    time::Rate,
    Blocking,
};
use esp_println::println;

use esp32s3_doomlike_lab::display::{fill_frame, Ili9341Bus, Ili9341Display, PixelSink};
use esp32s3_doomlike_lab::game::{Game, Phase};
use esp32s3_doomlike_lab::input::{InputState, RawButtons};
use esp32s3_doomlike_lab::render::{Renderer, HEIGHT, WIDTH};
use esp32s3_doomlike_lab::assets;

/// Internal 160×120 RGB565 render target (static — too large for the main stack).
static mut FRAME: [u16; WIDTH * HEIGHT] = [0; WIDTH * HEIGHT];

/// Hardware SPI + GPIO CS/DC/RESET bridge for the ILI9341.
struct HwBus {
    spi: Spi<'static, Blocking>,
    cs: Output<'static>,
    dc: Output<'static>,
    _reset: Output<'static>,
    delay: Delay,
}

impl HwBus {
    fn select(&mut self) {
        self.cs.set_low();
    }
    fn deselect(&mut self) {
        self.cs.set_high();
    }
}

impl Ili9341Bus for HwBus {
    type Error = ();

    fn write_command(&mut self, cmd: u8) -> Result<(), ()> {
        self.select();
        self.dc.set_low();
        self.spi.write(&[cmd]).map_err(|_| ())?;
        self.deselect();
        Ok(())
    }

    fn write_data(&mut self, data: &[u8]) -> Result<(), ()> {
        self.select();
        self.dc.set_high();
        self.spi.write(data).map_err(|_| ())?;
        self.deselect();
        Ok(())
    }

    fn delay_ms(&mut self, ms: u32) {
        self.delay.delay_millis(ms);
    }
}

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    println!("DOOMLIKE_BOOT");

    // Buttons: active-low with internal pull-ups (GPIO1..GPIO6).
    let btn_cfg = InputConfig::default().with_pull(Pull::Up);
    let btn_forward = Input::new(p.GPIO1, btn_cfg);
    let btn_backward = Input::new(p.GPIO2, btn_cfg);
    let btn_left = Input::new(p.GPIO3, btn_cfg);
    let btn_right = Input::new(p.GPIO4, btn_cfg);
    let btn_fire = Input::new(p.GPIO5, btn_cfg);
    let btn_use = Input::new(p.GPIO6, btn_cfg);

    // SPI2: SCLK=GPIO12, MOSI=GPIO13. CS/DC/RESET are GPIO bit-banged.
    let spi = Spi::new(
        p.SPI2,
        SpiConfig::default().with_frequency(Rate::from_mhz(10)),
    )
    .unwrap()
    .with_sck(p.GPIO12)
    .with_mosi(p.GPIO13);

    let cs = Output::new(p.GPIO10, Level::High, OutputConfig::default());
    let dc = Output::new(p.GPIO11, Level::High, OutputConfig::default());
    let mut reset = Output::new(p.GPIO14, Level::High, OutputConfig::default());

    // Hardware reset pulse before the command sequence.
    reset.set_low();
    delay.delay_millis(10);
    reset.set_high();
    delay.delay_millis(10);

    let bus = HwBus {
        spi,
        cs,
        dc,
        _reset: reset,
        delay,
    };
    let mut display = Ili9341Display::new(bus);

    let frame = unsafe { &mut *core::ptr::addr_of_mut!(FRAME) };

    if display.init().is_err() {
        fill_frame(frame, assets::rgb565(255, 0, 0));
        let _ = display.present_2x(frame);
        println!("DOOMLIKE_DISPLAY_ERROR");
        loop {
            core::hint::spin_loop();
        }
    }

    // Red diagnostic frame proves the panel path before gameplay starts.
    fill_frame(frame, assets::rgb565(255, 0, 0));
    if display.present_2x(frame).is_err() {
        println!("DOOMLIKE_DISPLAY_ERROR");
        loop {
            core::hint::spin_loop();
        }
    }

    let mut game = Game::new();
    let mut renderer = Renderer::new();
    let mut input = InputState::new();
    let mut prev_phase = Phase::Playing;
    let mut ready_printed = false;

    loop {
        let raw = RawButtons::from_active_low(
            btn_forward.is_high(),
            btn_backward.is_high(),
            btn_left.is_high(),
            btn_right.is_high(),
            btn_fire.is_high(),
            btn_use.is_high(),
        );
        let actions = input.update(raw);
        game.tick(actions);

        if game.phase != prev_phase {
            match game.phase {
                Phase::Won => println!("DOOMLIKE_WON"),
                Phase::Dead => println!("DOOMLIKE_DEAD"),
                Phase::Playing => {}
            }
            prev_phase = game.phase;
        }

        renderer.render(&game, frame);
        let _ = display.present_2x(frame);

        if !ready_printed {
            println!("DOOMLIKE_READY");
            ready_printed = true;
        }
    }
}
