//! A selected host UART must not inject bytes into other UARTs on the MCU.
mod common;

use labwired_core::session::{AddrOrSymbol, OpenOptions, Session};
use labwired_core::system::builder::*;

fn open(selected: Option<&str>) -> anyhow::Result<Session> {
    let f = common::bare_chip_fixture("stm32f103", "tests/fixtures/tier1/stm32f103.elf");
    let mut session = Session::open(
        BuildRequest {
            chip: &f.chip,
            system: &f.manifest,
            firmware: FirmwareSource::Elf(&f.fw),
            boot: BootMode::FastBoot,
            blobs: &BlobMap::new(),
            options: BuildOptions {
                uart_rx: selected.map(str::to_owned),
                ..Default::default()
            },
        },
        OpenOptions::default(),
    )?;
    // USART1's bus window reads zero until its APB2 clock is enabled. Enable
    // it so checking an unselected port can actually detect stray input.
    session.write_u32(AddrOrSymbol::Addr(0x4002_1018), 1 << 14)?;
    Ok(session)
}

#[test]
fn send_reaches_only_selected_uart_and_survives_restore() {
    let mut s = open(Some("uart2")).unwrap();
    let before = s.snapshot();
    s.send(b"A");
    // Read the actual UART RX queues through the firmware's data registers.
    assert_eq!(
        s.read_u32(AddrOrSymbol::Addr(0x4000_4404)).unwrap(),
        u32::from(b'A')
    );
    assert_eq!(s.read_u32(AddrOrSymbol::Addr(0x4001_3804)).unwrap(), 0);
    assert_eq!(s.read_u32(AddrOrSymbol::Addr(0x4000_4804)).unwrap(), 0);
    s.restore(&before).unwrap();
    s.send(b"B");
    assert_eq!(
        s.read_u32(AddrOrSymbol::Addr(0x4000_4404)).unwrap(),
        u32::from(b'B')
    );
    assert_eq!(s.read_u32(AddrOrSymbol::Addr(0x4001_3804)).unwrap(), 0);
}

#[test]
fn missing_uart_rx_is_rejected_at_construction() {
    let error = open(Some("not_a_uart"))
        .err()
        .expect("unknown UART must fail");
    assert!(error.to_string().contains("not_a_uart"));
}

#[test]
fn default_keeps_existing_broadcast_contract() {
    let mut s = open(None).unwrap();
    s.send(b"C");
    assert_eq!(
        s.read_u32(AddrOrSymbol::Addr(0x4000_4404)).unwrap(),
        u32::from(b'C')
    );
    assert_eq!(
        s.read_u32(AddrOrSymbol::Addr(0x4001_3804)).unwrap(),
        u32::from(b'C')
    );
}
