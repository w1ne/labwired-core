// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Chip-level checks of the CYW43439 gSPI engine: the host side is the byte
//! stream the pico-sdk driver puts on the wire (DMA BSWAP + MSB-first PIO
//! shift = memory byte order), clocked one bit per rising edge.

use super::*;

/// Host side of one gSPI transaction: CS low, clock out `tx` bytes MSB
/// first, then clock `rx_len` response bytes in, CS high.
fn transfer(chip: &mut Cyw43439Chip, tx: &[u8], rx_len: usize) -> Vec<u8> {
    chip.chip_select(true);
    for &byte in tx {
        for bit in (0..8).rev() {
            assert_eq!(chip.clock_rise(byte & (1 << bit) != 0), None);
        }
    }
    let mut rx = Vec::with_capacity(rx_len);
    for _ in 0..rx_len {
        let mut byte = 0u8;
        for _ in 0..8 {
            let bit = chip.clock_rise(false).expect("chip drives DIO on a read");
            byte = (byte << 1) | u8::from(bit);
        }
        rx.push(byte);
    }
    chip.chip_select(false);
    rx
}

fn make_cmd(write: bool, func: u32, addr: u32, len: u32) -> u32 {
    (u32::from(write) << 31) | (1 << 30) | (func << 28) | ((addr & 0x1_FFFF) << 11) | len
}

/// pico-sdk `SWAP32` (rev16): the 16-bit start-up word order.
fn swap16x2(v: u32) -> u32 {
    ((v & 0x00FF_00FF) << 8) | ((v & 0xFF00_FF00) >> 8)
}

fn read_u32_swap(chip: &mut Cyw43439Chip, func: u32, addr: u32) -> u32 {
    let cmd = swap16x2(make_cmd(false, func, addr, 4)).to_le_bytes();
    let rx = transfer(chip, &cmd, 4);
    swap16x2(u32::from_le_bytes(rx.try_into().unwrap()))
}

fn write_u32_swap(chip: &mut Cyw43439Chip, func: u32, addr: u32, val: u32) {
    let mut tx = swap16x2(make_cmd(true, func, addr, 4))
        .to_le_bytes()
        .to_vec();
    tx.extend_from_slice(&swap16x2(val).to_le_bytes());
    transfer(chip, &tx, 0);
}

fn write_reg(chip: &mut Cyw43439Chip, func: u32, addr: u32, len: u32, val: u32) {
    let mut tx = make_cmd(true, func, addr, len).to_le_bytes().to_vec();
    tx.extend_from_slice(&val.to_le_bytes());
    transfer(chip, &tx, 0);
}

fn read_reg(chip: &mut Cyw43439Chip, func: u32, addr: u32, len: u32) -> u32 {
    let pad = if func == 1 { 16 } else { 0 };
    let rx = transfer(
        chip,
        &make_cmd(false, func, addr, len).to_le_bytes(),
        pad + 4,
    );
    u32::from_le_bytes(rx[pad..pad + 4].try_into().unwrap())
}

const BUS_CTRL_32: u32 = 0x01 | 0x02 | 0x10 | 0x80 | 0x20 | (4 << 8) | (0x02 << 16);

fn bus_up(chip: &mut Cyw43439Chip) {
    assert_eq!(read_u32_swap(chip, 0, 0x14), TEST_PATTERN);
    write_u32_swap(chip, 0, 0x00, BUS_CTRL_32);
    write_reg(chip, 0, 0x1D, 1, 16);
    write_reg(chip, 0, 0x06, 2, 0x00BE);
}

fn backplane_write(chip: &mut Cyw43439Chip, addr: u32, len: u32, val: u32) {
    write_reg(chip, 1, SDIO_BACKPLANE_ADDRESS_LOW, 1, (addr >> 8) & 0xFF);
    write_reg(chip, 1, SDIO_BACKPLANE_ADDRESS_MID, 1, (addr >> 16) & 0xFF);
    write_reg(chip, 1, SDIO_BACKPLANE_ADDRESS_HIGH, 1, addr >> 24);
    write_reg(chip, 1, (addr & 0x7FFF) | 0x8000, len, val);
}

fn backplane_read(chip: &mut Cyw43439Chip, addr: u32, len: u32) -> u32 {
    write_reg(chip, 1, SDIO_BACKPLANE_ADDRESS_LOW, 1, (addr >> 8) & 0xFF);
    write_reg(chip, 1, SDIO_BACKPLANE_ADDRESS_MID, 1, (addr >> 16) & 0xFF);
    write_reg(chip, 1, SDIO_BACKPLANE_ADDRESS_HIGH, 1, addr >> 24);
    read_reg(chip, 1, (addr & 0x7FFF) | 0x8000, len)
}

/// Download a stand-in image shaped like the 43439A0 one (initial stack
/// pointer word 0, reset vector 0x1465) and the NVRAM token,
/// then release the WLAN core the way `reset_device_core` does.
fn boot(chip: &mut Cyw43439Chip) {
    backplane_write(chip, 4, 4, 0x0000_1465);
    let words = 0x100u32;
    backplane_write(
        chip,
        (RAM_SIZE - 4) as u32,
        4,
        ((!words & 0xFFFF) << 16) | words,
    );
    let wrapper = WLAN_ARMCM3_WRAPPER;
    backplane_write(chip, wrapper + AI_IOCTRL_OFFSET, 1, 0x3);
    backplane_write(chip, wrapper + AI_RESETCTRL_OFFSET, 1, 0);
    backplane_write(chip, wrapper + AI_IOCTRL_OFFSET, 1, 0x1);
}

fn sdpcm_ioctl(seq: u8, id: u16, kind: u32, cmd: u32, payload: &[u8]) -> Vec<u8> {
    let size = 12 + 16 + payload.len();
    let mut f = Vec::new();
    f.extend_from_slice(&(size as u16).to_le_bytes());
    f.extend_from_slice(&(!(size as u16)).to_le_bytes());
    f.extend_from_slice(&[seq, 0, 0, 12, 0, 0, 0, 0]);
    f.extend_from_slice(&cmd.to_le_bytes());
    f.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    f.extend_from_slice(&((u32::from(id) << 16) | kind).to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(payload);
    while f.len() % 4 != 0 {
        f.push(0);
    }
    f
}

fn send_frame(chip: &mut Cyw43439Chip, frame: &[u8]) {
    let mut tx = make_cmd(true, 2, 0, frame.len() as u32)
        .to_le_bytes()
        .to_vec();
    tx.extend_from_slice(frame);
    transfer(chip, &tx, 0);
}

fn read_frame(chip: &mut Cyw43439Chip) -> Vec<u8> {
    let status = read_reg(chip, 0, 0x08, 4);
    assert_ne!(status & STATUS_F2_PKT_AVAILABLE, 0, "a response waits");
    let len = (status >> 9) & 0x7FF;
    transfer(
        chip,
        &make_cmd(false, 2, 0, len).to_le_bytes(),
        len.next_multiple_of(4) as usize,
    )
}

#[test]
fn test_register_reads_feedbead_in_16_bit_start_up_order() {
    let mut chip = Cyw43439Chip::default();
    assert_eq!(read_u32_swap(&mut chip, 0, 0x14), 0xFEED_BEAD);
}

#[test]
fn bus_control_switches_to_32_bit_words() {
    let mut chip = Cyw43439Chip::default();
    bus_up(&mut chip);
    assert_eq!(read_reg(&mut chip, 0, 0x14, 4), 0xFEED_BEAD);
    assert_eq!(read_reg(&mut chip, 0, 0x00, 4) & 0xFF, BUS_CTRL_32 & 0xFF);
}

#[test]
fn backplane_reads_skip_the_f1_response_delay_and_cores_start_in_reset() {
    let mut chip = Cyw43439Chip::default();
    bus_up(&mut chip);
    let reset = backplane_read(&mut chip, WLAN_ARMCM3_WRAPPER + AI_RESETCTRL_OFFSET, 1);
    assert_eq!(reset & 1, 1);
    assert_eq!(
        backplane_read(&mut chip, CHIPCOMMON_BASE_ADDRESS, 4) & 0xFFFF,
        43439
    );
    // ALP is available on request; HT waits for the firmware.
    write_reg(&mut chip, 1, SDIO_CHIP_CLOCK_CSR, 1, 0x08);
    let csr = read_reg(&mut chip, 1, SDIO_CHIP_CLOCK_CSR, 1) & 0xFF;
    assert_eq!(csr & 0xC0, 0x40);
}

#[test]
fn ram_download_reads_back_through_the_window() {
    let mut chip = Cyw43439Chip::default();
    bus_up(&mut chip);
    backplane_write(&mut chip, 0x0001_2340, 4, 0xA5A5_1234);
    assert_eq!(backplane_read(&mut chip, 0x0001_2340, 4), 0xA5A5_1234);
}

#[test]
fn firmware_needs_an_image_and_the_nvram_token() {
    let mut chip = Cyw43439Chip::default();
    bus_up(&mut chip);
    let wrapper = WLAN_ARMCM3_WRAPPER;
    backplane_write(&mut chip, wrapper + AI_IOCTRL_OFFSET, 1, 0x1);
    backplane_write(&mut chip, wrapper + AI_RESETCTRL_OFFSET, 1, 0);
    assert!(!chip.firmware_running(), "empty RAM does not boot");
    let mut chip = Cyw43439Chip::default();
    bus_up(&mut chip);
    boot(&mut chip);
    assert!(chip.firmware_running());
    assert_ne!(read_reg(&mut chip, 0, 0x08, 4) & STATUS_F2_RX_READY, 0);
    let csr = read_reg(&mut chip, 1, SDIO_CHIP_CLOCK_CSR, 1) & 0xFF;
    assert_ne!(csr & u32::from(SBSDIO_HT_AVAIL), 0);
}

#[test]
fn gpioout_ioctl_drives_wl_gpio0_and_is_answered_with_credit() {
    let mut chip = Cyw43439Chip::default();
    bus_up(&mut chip);
    boot(&mut chip);
    assert!(!chip.host_wake());
    let mut payload = b"gpioout\0".to_vec();
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    send_frame(&mut chip, &sdpcm_ioctl(0, 1, 2, WLC_SET_VAR, &payload));
    assert!(chip.wl_gpio(0), "LED on");
    assert!(chip.host_wake(), "F2 packet available raises host-wake");
    let resp = read_frame(&mut chip);
    let size = u16::from_le_bytes([resp[0], resp[1]]);
    assert_eq!(size, !u16::from_le_bytes([resp[2], resp[3]]));
    assert_eq!(resp[5] & 0xF, 0, "control channel");
    assert_eq!(resp[9], 8, "credit = host seq + window");
    let flags = u32::from_le_bytes(resp[20..24].try_into().unwrap());
    assert_eq!(flags >> 16, 1, "same ioctl id");
    assert!(!chip.host_wake(), "drained");
    let mut payload = b"gpioout\0".to_vec();
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    send_frame(&mut chip, &sdpcm_ioctl(1, 2, 2, WLC_SET_VAR, &payload));
    assert!(!chip.wl_gpio(0), "LED off");
    assert_eq!(chip.led_edges, 2);
}

#[test]
fn cur_etheraddr_reads_the_configured_mac() {
    let mut chip = Cyw43439Chip::new([0x28, 0xCD, 0xC1, 0x12, 0x34, 0x56]);
    bus_up(&mut chip);
    boot(&mut chip);
    let mut payload = b"cur_etheraddr\0".to_vec();
    payload.extend_from_slice(&[0; 6]);
    send_frame(&mut chip, &sdpcm_ioctl(0, 1, 0, WLC_GET_VAR, &payload));
    let resp = read_frame(&mut chip);
    assert_eq!(&resp[28..34], &[0x28, 0xCD, 0xC1, 0x12, 0x34, 0x56]);
}

#[test]
fn frames_before_the_firmware_runs_are_ignored() {
    let mut chip = Cyw43439Chip::default();
    bus_up(&mut chip);
    let mut payload = b"gpioout\0".to_vec();
    payload.extend_from_slice(&[1, 0, 0, 0, 1, 0, 0, 0]);
    send_frame(&mut chip, &sdpcm_ioctl(0, 1, 2, WLC_SET_VAR, &payload));
    assert!(!chip.wl_gpio(0));
    assert_eq!(read_reg(&mut chip, 0, 0x08, 4) & STATUS_F2_PKT_AVAILABLE, 0);
}
