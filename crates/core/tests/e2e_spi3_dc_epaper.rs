// Proof: a tri-color e-paper panel paints when driven through the REAL ESP32
// SPI3 peripheral registers (FIFO + MOSI_DLEN + CMD.USR) with command/data
// framed by the REAL DC GPIO line — no GxEPD2 panel-bypass thunk, no direct
// command_byte/data_byte injection. This is the de-thunked path: firmware sets
// the DC GPIO and clocks bytes out over SPI3; the bus latches DC before each
// transfer and drains the FIFO into the attached panel via kick_user_transaction.
//
// This is the register-level proof of the de-thunked path. The full-firmware
// counterpart (tests/e2e_labwired_ereader.rs) runs the real compiled GxEPD2
// ELF through this same machinery — both gxepd_write_command/data bypass thunks
// are now deleted (FIDELITY.md §A).

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::{Ssd1680Tricolor290, Uc8151dTricolor290};
use labwired_core::peripherals::esp32::spi::Esp32Spi;
use labwired_core::peripherals::spi::SpiDevice;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::Bus;

const SPI3_BASE: u64 = 0x3FF6_5000;
const GPIO_BASE: u64 = 0x3FF4_4000;
const GPIO_OUT_W1TS: u64 = 0x08;
const GPIO_OUT_W1TC: u64 = 0x0C;
const DC_PIN: &str = "GPIO17";
const DC_BIT: u32 = 1 << 17;

// SPI3 register offsets (see peripherals/esp32/spi.rs).
const SPI_CMD: u64 = 0x00; // bit 18 = USR (start)
const SPI_USER: u64 = 0x1C; // bit 27 = USR_MOSI
const SPI_MOSI_DLEN: u64 = 0x28; // bit length - 1
const SPI_W0: u64 = 0x80; // FIFO word 0

/// Drive one byte exactly as Arduino-ESP32 firmware does: set the DC GPIO, load
/// the FIFO, size the MOSI phase to 8 bits, and fire CMD.USR. The bus latches
/// the DC level (from the real GPIO output reg) on these SPI writes, so by the
/// time CMD.USR drains the FIFO the panel sees correct command/data framing.
fn spi3_xfer(bus: &mut SystemBus, dc_high: bool, byte: u8) {
    if dc_high {
        bus.write_u32(GPIO_BASE + GPIO_OUT_W1TS, DC_BIT).unwrap(); // DC high = data
    } else {
        bus.write_u32(GPIO_BASE + GPIO_OUT_W1TC, DC_BIT).unwrap(); // DC low = command
    }
    bus.write_u32(SPI3_BASE + SPI_W0, byte as u32).unwrap();
    bus.write_u32(SPI3_BASE + SPI_MOSI_DLEN, 7).unwrap();
    bus.write_u32(SPI3_BASE + SPI_USER, 1 << 27).unwrap();
    bus.write_u32(SPI3_BASE + SPI_CMD, 1 << 18).unwrap();
}

fn cmd(bus: &mut SystemBus, b: u8) {
    spi3_xfer(bus, false, b);
}
fn dat(bus: &mut SystemBus, b: u8) {
    spi3_xfer(bus, true, b);
}

fn attach_panel_with_dc(bus: &mut SystemBus, panel: Box<dyn SpiDevice>) -> usize {
    let dc_src = SystemBus::resolve_pin_odr_pub(bus, DC_PIN)
        .expect("GPIO17 must resolve to the ESP32 GPIO OUT register");
    let spi3_idx = bus
        .find_peripheral_index_by_name("spi3")
        .expect("configure_xtensa_esp32 registers spi3");
    let mut panel = panel;
    panel.set_dc_source(dc_src.0, dc_src.1);
    bus.attach_spi_device("spi3", panel)
        .expect("spi3 is an Esp32Spi controller");
    bus.refresh_peripheral_index();
    spi3_idx
}

#[test]
fn uc8151d_paints_over_real_spi3_and_dc() {
    let mut bus = SystemBus::new();
    let _cpu = configure_xtensa_esp32(&mut bus);
    let spi3_idx = attach_panel_with_dc(
        &mut bus,
        Box::new(Uc8151dTricolor290::new("GPIO5").with_dc_pin(DC_PIN)),
    );

    // Real GxEPD2 (GxEPD2_290_C90c / UC8151D) init + refresh stream — same bytes
    // as uc8151d::tests::ereader_init_powers_panel_on, but clocked through SPI3.
    cmd(&mut bus, 0x00); // PSR
    dat(&mut bus, 0x8F);
    cmd(&mut bus, 0x61); // TRES 128 x 296
    dat(&mut bus, 0x80);
    dat(&mut bus, 0x01);
    dat(&mut bus, 0x28);
    cmd(&mut bus, 0x50); // CDI
    dat(&mut bus, 0x77);
    cmd(&mut bus, 0x04); // PON
    cmd(&mut bus, 0x12); // DRF (display refresh)

    let any = bus.peripherals[spi3_idx].dev.as_any().unwrap();
    let spi3 = any.downcast_ref::<Esp32Spi>().unwrap();
    let panel = spi3.attached_devices[0]
        .as_any()
        .unwrap()
        .downcast_ref::<Uc8151dTricolor290>()
        .unwrap();
    assert!(
        panel.power_on(),
        "PON clocked over real SPI3 with DC-low framing must power the panel on"
    );
    assert_eq!(
        panel.refresh_generation(),
        1,
        "DRF over real SPI3 + real DC must drive exactly one refresh"
    );
}

#[test]
fn ssd1680_paints_over_real_spi3_and_dc() {
    let mut bus = SystemBus::new();
    let _cpu = configure_xtensa_esp32(&mut bus);
    let spi3_idx = attach_panel_with_dc(
        &mut bus,
        Box::new(Ssd1680Tricolor290::new("GPIO5").with_dc_pin(DC_PIN)),
    );

    // Minimal SSD1680 (GxEPD2_290_T94) update: reset, data-entry, a 1-byte RAM
    // window, write one black byte, then 0x22/0x20 master activation = refresh.
    cmd(&mut bus, 0x12); // SWRESET
    cmd(&mut bus, 0x11); // data entry mode
    dat(&mut bus, 0x03);
    cmd(&mut bus, 0x44); // RAM-X window: byte 0..0
    dat(&mut bus, 0x00);
    dat(&mut bus, 0x00);
    cmd(&mut bus, 0x45); // RAM-Y window: row 0..0
    dat(&mut bus, 0x00);
    dat(&mut bus, 0x00);
    dat(&mut bus, 0x00);
    dat(&mut bus, 0x00);
    cmd(&mut bus, 0x4E); // RAM-X counter = 0
    dat(&mut bus, 0x00);
    cmd(&mut bus, 0x4F); // RAM-Y counter = 0
    dat(&mut bus, 0x00);
    dat(&mut bus, 0x00);
    cmd(&mut bus, 0x24); // write black RAM (window = 1 byte)
    dat(&mut bus, 0x00);
    cmd(&mut bus, 0x22); // display update control 2
    dat(&mut bus, 0xF7);
    cmd(&mut bus, 0x20); // master activation = refresh

    let any = bus.peripherals[spi3_idx].dev.as_any().unwrap();
    let spi3 = any.downcast_ref::<Esp32Spi>().unwrap();
    let panel = spi3.attached_devices[0]
        .as_any()
        .unwrap()
        .downcast_ref::<Ssd1680Tricolor290>()
        .unwrap();
    assert_eq!(
        panel.refresh_generation(),
        1,
        "0x20 master activation over real SPI3 + real DC must drive one refresh"
    );
}
