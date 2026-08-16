//! ESP32-S3 LCD_CAM i80 pixel streaming, end to end.
//!
//! Drives the **real** path a `esp_lcd` i80 transaction takes:
//!
//!   firmware MMIO → LCD_CAM (`LCD_CMD_VAL`, `LCD_USER.LCD_START`)
//!                 → GDMA outlink descriptor chain (`OUT_PERI_SEL = 5`)
//!                 → `Ili9341Parallel` framebuffer
//!
//! Nothing is called directly on the models: every register write goes through
//! `SystemBus` at the silicon addresses, so a wrong offset, a missing
//! registration or an unwired panel fails here. (The LC_DMA interrupt block
//! *was* modelled 0x30 too low, which made every i80 bring-up spin forever —
//! `interrupt_block_lives_at_0x64` pins it.)
//!
//! The register sequence below is the one ESP-IDF v5.3.1
//! `esp_lcd_panel_io_i80.c` emits; the comments name the driver call each write
//! comes from.

use labwired_config::SystemManifest;
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::esp32s3::lcd_cam::{Esp32s3LcdCam, LCD_CAM_BASE};
use labwired_core::system::xtensa::{
    attach_esp32_external_devices, configure_xtensa_esp32s3, Esp32s3Opts,
};
use labwired_core::Bus;

// ── LCD_CAM (0x6004_1000) ────────────────────────────────────────────────
const LCD_USER: u64 = LCD_CAM_BASE as u64 + 0x14;
const LCD_MISC: u64 = LCD_CAM_BASE as u64 + 0x18;
const LCD_CMD_VAL: u64 = LCD_CAM_BASE as u64 + 0x28;
const LC_DMA_INT_ENA: u64 = LCD_CAM_BASE as u64 + 0x64;
const LC_DMA_INT_ST: u64 = LCD_CAM_BASE as u64 + 0x6C;
const LC_DMA_INT_CLR: u64 = LCD_CAM_BASE as u64 + 0x70;

const LCD_2BYTE_EN: u32 = 1 << 23;
const LCD_DOUT: u32 = 1 << 24;
const LCD_CMD: u32 = 1 << 26;
const LCD_START: u32 = 1 << 27;
const EVENT_TRANS_DONE: u32 = 1 << 1;

/// `lcd_ll_set_dc_level(dev, idle=0, cmd=0, dummy=0, data=1)` — the level map
/// `esp_lcd_new_panel_io_i80` programs for an ILI9341: D/C low for the command
/// phase, high for the payload. Only `CD_DATA_SET` (bit 28) differs from idle.
const MISC_DC_LEVELS: u32 = 1 << 28;

// ── GDMA (0x6003_F000), channel 0 ────────────────────────────────────────
const GDMA_BASE: u64 = 0x6003_F000;
const OUT_INT_RAW: u64 = GDMA_BASE + 0x68;
const OUT_LINK: u64 = GDMA_BASE + 0x80;
const OUT_PERI_SEL: u64 = GDMA_BASE + 0xA8;
const OUT_LINK_START: u32 = 1 << 21;
const OUT_EOF: u32 = 1 << 1;
/// `GDMA_TRIG_PERIPH_LCD` on the S3.
const PERI_SEL_LCD: u32 = 5;
/// Mirrors `Esp32s3Gdma::LCD_BYTES_PER_TICK`; only used to phrase the
/// multi-tick assertion below in the units the model works in.
const LCD_BYTES_PER_TICK: usize = 4096;

// Internal SRAM: descriptors must live here (the outlink register carries only
// address bits [19:0], the top 12 are implicitly 0x3FC).
const DESC_ADDR: u64 = 0x3FC9_0000;
const DESC2_ADDR: u64 = 0x3FC9_0100;
const DESC3_ADDR: u64 = 0x3FC9_0200;
const BUF_ADDR: u64 = 0x3FC9_1000;

const DESC_OWNER_DMA: u32 = 1 << 31;
const DESC_SUC_EOF: u32 = 1 << 30;

/// ILI9341 landscape MADCTL (`MV | BGR`) — what the firmware sends. The panel
/// is then addressed 320 wide × 240 high.
const MADCTL_LANDSCAPE: u8 = 0x28;

fn build_bus() -> SystemBus {
    let mut bus = SystemBus::new();
    let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "s3-i80-panel"
chip: "../chips/esp32s3.yaml"
external_devices:
  - id: "tft"
    type: "ili9341-16bit"
    connection: "gpio"
    config:
      cs_pin: "GPIO21"
      rs_pin: "GPIO18"
      wr_pin: "GPIO17"
      rd_pin: "GPIO47"
      rst_pin: "GPIO38"
      db0_pin: "GPIO1"
      db1_pin: "GPIO2"
      db2_pin: "GPIO3"
      db3_pin: "GPIO4"
      db4_pin: "GPIO5"
      db5_pin: "GPIO6"
      db6_pin: "GPIO7"
      db7_pin: "GPIO8"
      db8_pin: "GPIO9"
      db9_pin: "GPIO10"
      db10_pin: "GPIO11"
      db11_pin: "GPIO12"
      db12_pin: "GPIO13"
      db13_pin: "GPIO14"
      db14_pin: "GPIO15"
      db15_pin: "GPIO16"
board_io: []
"#,
    )
    .expect("parse manifest");
    attach_esp32_external_devices(&mut bus, &manifest).expect("attach parallel panel");
    assert_eq!(bus.ili9341_parallel.len(), 1, "panel attached");
    // The kit must bind the panel to LCD_CAM as well as to the GPIO observer;
    // without that binding the i80 path has nothing to paint and every
    // assertion below would fail for the wrong reason.
    let lcd_idx = bus
        .find_peripheral_index_by_name("lcd_cam")
        .expect("lcd_cam registered");
    let panels = bus.peripherals[lcd_idx]
        .dev
        .as_any()
        .and_then(|a| a.downcast_ref::<Esp32s3LcdCam>())
        .expect("lcd_cam is the LCD_CAM model")
        .panel_count();
    assert_eq!(panels, 1, "panel bound to LCD_CAM's i80 output");
    bus
}

/// One descriptor: DMA-owned, `suc_eof`, `len` bytes out of `buf`.
fn write_desc(bus: &mut SystemBus, at: u64, len: u32, buf: u64, next: u64, eof: bool) {
    let dw0 = DESC_OWNER_DMA | if eof { DESC_SUC_EOF } else { 0 } | (len << 12) | len;
    bus.write_u32(at, dw0).unwrap();
    bus.write_u32(at + 4, buf as u32).unwrap();
    bus.write_u32(at + 8, next as u32).unwrap();
}

/// Advance the bus until `done()` or `budget` ticks elapse. Returns the tick
/// count actually used.
fn tick_until(
    bus: &mut SystemBus,
    budget: usize,
    mut done: impl FnMut(&mut SystemBus) -> bool,
) -> usize {
    for n in 0..budget {
        if done(bus) {
            return n;
        }
        bus.tick_peripherals_with_costs();
    }
    assert!(done(bus), "condition not reached within {budget} ticks");
    budget
}

fn trans_done(bus: &mut SystemBus) -> bool {
    bus.read_u32(LC_DMA_INT_ST).unwrap() & EVENT_TRANS_DONE != 0
}

/// Send one command byte with no parameters — command phase only, no DMA.
fn send_cmd(bus: &mut SystemBus, cmd: u8) {
    bus.write_u32(LC_DMA_INT_CLR, EVENT_TRANS_DONE).unwrap();
    bus.write_u32(LCD_CMD_VAL, cmd as u32).unwrap();
    bus.write_u32(LCD_USER, LCD_2BYTE_EN | LCD_CMD | LCD_START)
        .unwrap();
    tick_until(bus, 64, trans_done);
}

/// Send a command plus `params` 8-bit parameters. The i80 driver widens each
/// 8-bit parameter to one 16-bit bus cycle in `format_buffer` (low byte carries
/// the value), then streams the buffer through the same outlink chain the pixel
/// path uses.
fn send_cmd_params(bus: &mut SystemBus, cmd: u8, params: &[u8]) {
    let mut widened = Vec::with_capacity(params.len() * 2);
    for &p in params {
        widened.push(p);
        widened.push(0);
    }
    send_dma_transaction(bus, cmd, &widened);
}

/// The full `lcd_start_transaction` sequence: mount the payload on the outlink
/// chain, `gdma_start`, then `lcd_ll_start`.
fn send_dma_transaction(bus: &mut SystemBus, cmd: u8, payload: &[u8]) {
    assert!(
        payload.len() <= 4095,
        "one descriptor is enough for this test"
    );
    bus.write_u32(LC_DMA_INT_CLR, EVENT_TRANS_DONE).unwrap();
    for (i, &b) in payload.iter().enumerate() {
        bus.write_u8(BUF_ADDR + i as u64, b).unwrap();
    }
    write_desc(bus, DESC_ADDR, payload.len() as u32, BUF_ADDR, 0, true);

    // gdma_start(chan, dma_nodes)
    bus.write_u32(OUT_PERI_SEL, PERI_SEL_LCD).unwrap();
    bus.write_u32(OUT_LINK, OUT_LINK_START | (DESC_ADDR as u32 & 0x000F_FFFF))
        .unwrap();
    // lcd_ll_set_command + lcd_ll_set_phase_cycles(1, 0, 1) + lcd_ll_start
    bus.write_u32(LCD_CMD_VAL, cmd as u32).unwrap();
    bus.write_u32(LCD_USER, LCD_2BYTE_EN | LCD_CMD | LCD_DOUT | LCD_START)
        .unwrap();
    tick_until(bus, 4096, trans_done);
}

/// Bring the panel up the way the firmware does: MADCTL, 16 bpp, display on.
fn panel_init(bus: &mut SystemBus) {
    bus.write_u32(LCD_MISC, MISC_DC_LEVELS).unwrap();
    bus.write_u32(LC_DMA_INT_ENA, EVENT_TRANS_DONE).unwrap();
    send_cmd_params(bus, 0x36, &[MADCTL_LANDSCAPE]); // MADCTL
    send_cmd_params(bus, 0x3A, &[0x55]); // COLMOD: 16 bit/px
    send_cmd(bus, 0x29); // DISPON
}

fn set_window(bus: &mut SystemBus, x0: u16, y0: u16, x1: u16, y1: u16) {
    send_cmd_params(
        bus,
        0x2A,
        &[(x0 >> 8) as u8, x0 as u8, (x1 >> 8) as u8, x1 as u8],
    );
    send_cmd_params(
        bus,
        0x2B,
        &[(y0 >> 8) as u8, y0 as u8, (y1 >> 8) as u8, y1 as u8],
    );
}

/// The whole point: `LC_DMA_INT_*` at 0x64..0x70. Modelled 0x30 lower, INT_ENA
/// writes fell into accept-and-ignore, INT_ST read a constant 0, and
/// `esp_lcd_new_i80_bus`'s TRANS_DONE poll never returned.
#[test]
fn interrupt_block_lives_at_0x64() {
    let mut bus = build_bus();
    bus.write_u32(LC_DMA_INT_ENA, EVENT_TRANS_DONE).unwrap();
    assert_eq!(
        bus.read_u32(LC_DMA_INT_ENA).unwrap(),
        EVENT_TRANS_DONE,
        "LC_DMA_INT_ENA must round-trip at +0x64"
    );
    assert_eq!(
        bus.read_u32(LC_DMA_INT_ST).unwrap(),
        0,
        "nothing latched yet"
    );

    // A command-only transaction must light TRANS_DONE in INT_ST.
    bus.write_u32(LCD_USER, LCD_2BYTE_EN | LCD_CMD | LCD_START)
        .unwrap();
    tick_until(&mut bus, 64, trans_done);
    assert_eq!(
        bus.read_u32(LC_DMA_INT_ST).unwrap() & EVENT_TRANS_DONE,
        EVENT_TRANS_DONE,
        "TRANS_DONE must be visible through INT_ST at +0x6C"
    );
    // W1C at +0x70.
    bus.write_u32(LC_DMA_INT_CLR, EVENT_TRANS_DONE).unwrap();
    assert_eq!(bus.read_u32(LC_DMA_INT_ST).unwrap(), 0, "INT_CLR at +0x70");
}

/// A DOUT transaction paints its payload into the panel framebuffer at the
/// window the CASET/PASET commands opened — the whole chain, from a bus write
/// to a pixel.
#[test]
fn i80_transaction_streams_pixels_into_the_panel_framebuffer() {
    let mut bus = build_bus();
    panel_init(&mut bus);

    // A 4×3 window at (10, 20) in landscape (logical) coordinates.
    const X0: u16 = 10;
    const Y0: u16 = 20;
    const W: u16 = 4;
    const H: u16 = 3;
    set_window(&mut bus, X0, Y0, X0 + W - 1, Y0 + H - 1);

    // Distinct RGB565 per pixel so a transposed / shifted / duplicated write
    // cannot pass: value = 0x1000 + index.
    let pixels: Vec<u16> = (0..(W * H)).map(|i| 0x1000 + i).collect();
    let mut payload = Vec::new();
    for p in &pixels {
        payload.extend_from_slice(&p.to_le_bytes()); // DMA reads memory LE
    }
    send_dma_transaction(&mut bus, 0x2C, &payload); // RAMWR

    let panel = &bus.ili9341_parallel[0];
    let (lw, lh) = panel.logical_dimensions();
    assert_eq!((lw, lh), (320, 240), "MADCTL 0x28 gives landscape 320x240");
    let fb = panel.oriented_framebuffer();

    // Region bound: every painted pixel is inside the window, in order.
    for (i, want) in pixels.iter().enumerate() {
        let col = X0 as usize + i % W as usize;
        let row = Y0 as usize + i / W as usize;
        let at = (row * lw + col) * 2;
        let got = u16::from_be_bytes([fb[at], fb[at + 1]]);
        assert_eq!(
            got, *want,
            "pixel {i} at ({col},{row}) — panel holds {got:#06x}, streamed {want:#06x}"
        );
    }
    // Ink bound: exactly the window was touched, nothing bled outside it.
    let mut lit = 0usize;
    for row in 0..lh {
        for col in 0..lw {
            let at = (row * lw + col) * 2;
            if fb[at] != 0 || fb[at + 1] != 0 {
                lit += 1;
                let inside = (X0 as usize..X0 as usize + W as usize).contains(&col)
                    && (Y0 as usize..Y0 as usize + H as usize).contains(&row);
                assert!(inside, "pixel outside the window painted at ({col},{row})");
            }
        }
    }
    assert_eq!(lit, (W * H) as usize, "exactly the window is lit");
    assert!(panel.display_on(), "DISPON reached the panel");
}

/// TRANS_DONE must stay low while the outlink chain is still streaming.
/// The driver polls it and then rebuilds the descriptors, so a transaction that
/// "completes" mid-transfer corrupts the next frame.
#[test]
fn trans_done_is_withheld_until_the_outlink_chain_drains() {
    let mut bus = build_bus();
    panel_init(&mut bus);
    set_window(&mut bus, 0, 0, 319, 239);

    // Three descriptors of 4000 bytes each — well past the per-tick budget, so
    // the transfer provably spans several ticks.
    let total = 12000usize;
    for i in 0..total {
        // 0x01..0xFF, never 0, so every byte moved shows up as ink.
        bus.write_u8(BUF_ADDR + i as u64, (i % 255 + 1) as u8)
            .unwrap();
    }
    write_desc(&mut bus, DESC_ADDR, 4000, BUF_ADDR, DESC2_ADDR, false);
    write_desc(
        &mut bus,
        DESC2_ADDR,
        4000,
        BUF_ADDR + 4000,
        DESC3_ADDR,
        false,
    );
    write_desc(&mut bus, DESC3_ADDR, 4000, BUF_ADDR + 8000, 0, true);

    bus.write_u32(LC_DMA_INT_CLR, EVENT_TRANS_DONE).unwrap();
    bus.write_u32(OUT_PERI_SEL, PERI_SEL_LCD).unwrap();
    bus.write_u32(OUT_LINK, OUT_LINK_START | (DESC_ADDR as u32 & 0x000F_FFFF))
        .unwrap();
    bus.write_u32(LCD_CMD_VAL, 0x2C).unwrap();
    bus.write_u32(LCD_USER, LCD_2BYTE_EN | LCD_CMD | LCD_DOUT | LCD_START)
        .unwrap();

    // One tick cannot move 8000 bytes: TRANS_DONE must still be low, and the
    // panel must already be part-painted (the transfer is genuinely running,
    // not merely stalled).
    bus.tick_peripherals_with_costs();
    assert!(
        !trans_done(&mut bus),
        "TRANS_DONE latched while the chain was still draining"
    );
    let partial = bus.ili9341_parallel[0].ink_bytes();
    assert!(partial > 0, "no pixels moved on the first tick");

    let ticks = 1 + tick_until(&mut bus, 4096, trans_done);
    assert!(
        ticks >= 3,
        "12 000 bytes at {LCD_BYTES_PER_TICK} B/tick must span >= 3 ticks, took {ticks}"
    );
    assert_eq!(
        bus.read_u32(OUT_INT_RAW).unwrap() & OUT_EOF,
        OUT_EOF,
        "GDMA must latch OUT_EOF when the chain drains"
    );
    assert!(
        bus.ili9341_parallel[0].ink_bytes() > partial,
        "the rest of the chain never reached the panel"
    );
}

/// The outlink walk must stop at `suc_eof`, not merely at `next == 0`. The i80
/// driver keeps a fixed descriptor pool and rewrites only the leading nodes, so
/// the tail still holds the *previous* transfer's lengths and buffers.
#[test]
fn outlink_walk_stops_at_suc_eof_not_at_the_end_of_the_pool() {
    let mut bus = build_bus();
    panel_init(&mut bus);
    set_window(&mut bus, 0, 0, 319, 239);

    // Node 0: 2 bytes = one pixel, flagged suc_eof, but still chained to a
    // stale node 1 that is DMA-owned with a big length (what a previous frame
    // would have left behind).
    // Pixel 0x1234 — both bytes non-zero, so "2 bytes of ink" is exactly one
    // pixel and cannot be confused with a half-written word.
    bus.write_u32(BUF_ADDR, 0x0000_3412).unwrap(); // 0x1234 little-endian
    for i in 0..512u64 {
        bus.write_u8(BUF_ADDR + 0x400 + i, 0xAB).unwrap();
    }
    write_desc(&mut bus, DESC_ADDR, 2, BUF_ADDR, DESC2_ADDR, true);
    write_desc(&mut bus, DESC2_ADDR, 512, BUF_ADDR + 0x400, 0, true);

    bus.write_u32(LC_DMA_INT_CLR, EVENT_TRANS_DONE).unwrap();
    bus.write_u32(OUT_PERI_SEL, PERI_SEL_LCD).unwrap();
    bus.write_u32(OUT_LINK, OUT_LINK_START | (DESC_ADDR as u32 & 0x000F_FFFF))
        .unwrap();
    bus.write_u32(LCD_CMD_VAL, 0x2C).unwrap();
    bus.write_u32(LCD_USER, LCD_2BYTE_EN | LCD_CMD | LCD_DOUT | LCD_START)
        .unwrap();
    tick_until(&mut bus, 64, trans_done);

    // Exactly one pixel: 2 non-zero bytes. The stale node would have added 256.
    assert_eq!(
        bus.ili9341_parallel[0].ink_bytes(),
        2,
        "walk ran past the suc_eof descriptor into the stale pool tail"
    );
}

/// The GPIO bit-bang path must keep working: a firmware that toggles the pads
/// itself never touches LCD_CAM, and the same panel model has to paint.
#[test]
fn gpio_bitbang_path_still_paints_without_lcd_cam() {
    let bus = build_bus();
    let panel = bus.ili9341_parallel[0].clone();
    let pins = *panel.pins();

    let strobe = |dc: bool, word: u16| {
        panel.on_gpio_edge(pins.rs, dc, 0);
        for bit in 0..16u8 {
            panel.on_gpio_edge(pins.db[bit as usize], (word >> bit) & 1 != 0, 0);
        }
        panel.on_gpio_edge(pins.wr, true, 0);
        panel.on_gpio_edge(pins.wr, false, 0);
        panel.on_gpio_edge(pins.wr, true, 0);
    };

    panel.on_gpio_edge(pins.cs, false, 0); // select
    strobe(false, 0x29); // DISPON
    strobe(false, 0x2C); // RAMWR
    strobe(true, 0x07E0); // one green pixel

    assert!(panel.display_on(), "DISPON via GPIO edges");
    let fb = panel.framebuffer();
    assert_eq!((fb[0], fb[1]), (0x07, 0xE0), "GPIO-driven pixel at origin");
    assert_eq!(panel.ink_bytes(), 2, "exactly one pixel painted");
}
