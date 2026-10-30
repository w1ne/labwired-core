use super::*;
use crate::sim_input::SimInput;

fn xfer(dev: &mut Rc522, bytes: &[u8]) -> Vec<u8> {
    dev.cs_select();
    let out: Vec<u8> = bytes.iter().map(|&b| dev.transfer(b)).collect();
    dev.cs_release();
    out
}

fn write_reg(dev: &mut Rc522, reg: u8, value: u8) {
    xfer(dev, &[reg << 1, value]);
}

fn write_fifo(dev: &mut Rc522, data: &[u8]) {
    let mut frame = Vec::with_capacity(1 + data.len());
    frame.push(REG_FIFODATA << 1);
    frame.extend_from_slice(data);
    xfer(dev, &frame);
}

fn read_reg(dev: &mut Rc522, reg: u8) -> u8 {
    xfer(dev, &[0x80 | (reg << 1), 0x00])[1]
}

fn read_fifo(dev: &mut Rc522, n: usize) -> Vec<u8> {
    let mut frame = vec![0x80 | (REG_FIFODATA << 1)];
    frame.extend(std::iter::repeat_n(0u8, n));
    xfer(dev, &frame)[1..].to_vec()
}

fn flush_fifo(dev: &mut Rc522) {
    write_reg(dev, REG_FIFOLEVEL, 0x80);
}

fn clear_comirq(dev: &mut Rc522) {
    write_reg(dev, REG_COMIRQ, 0x7F);
}

/// Mirror of MFRC522::PCD_TransceiveData for unit tests.
fn transceive(dev: &mut Rc522, send: &[u8], tx_last_bits: u8) -> (u8, Vec<u8>) {
    write_reg(dev, REG_COMMAND, CMD_IDLE);
    clear_comirq(dev);
    flush_fifo(dev);
    write_fifo(dev, send);
    write_reg(dev, REG_BITFRAMING, tx_last_bits);
    write_reg(dev, REG_COMMAND, CMD_TRANSCEIVE);
    write_reg(dev, REG_BITFRAMING, 0x80 | tx_last_bits);
    let irq = read_reg(dev, REG_COMIRQ);
    let level = read_reg(dev, REG_FIFOLEVEL) as usize;
    let data = if level > 0 {
        read_fifo(dev, level)
    } else {
        Vec::new()
    };
    (irq, data)
}

fn calc_crc(dev: &mut Rc522, data: &[u8]) -> [u8; 2] {
    write_reg(dev, REG_COMMAND, CMD_IDLE);
    write_reg(dev, REG_DIVIRQ, 0x04); // clear CRCIRq
    flush_fifo(dev);
    write_fifo(dev, data);
    write_reg(dev, REG_COMMAND, CMD_CALCCRC);
    let div = read_reg(dev, REG_DIVIRQ);
    assert_ne!(div & DIVIRQ_CRC, 0, "CalcCRC must raise CRCIRq");
    assert_eq!(
        read_reg(dev, REG_COMMAND) & 0x0F,
        CMD_IDLE,
        "CalcCRC completion returns CommandReg to Idle"
    );
    [
        read_reg(dev, REG_CRCRESULT_L),
        read_reg(dev, REG_CRCRESULT_H),
    ]
}

#[test]
fn version_register_is_0x92_and_read_only() {
    let mut d = Rc522::new("PA4");
    assert_eq!(read_reg(&mut d, REG_VERSION), 0x92);
    write_reg(&mut d, REG_VERSION, 0x00);
    assert_eq!(read_reg(&mut d, REG_VERSION), 0x92);
}

#[test]
fn soft_reset_returns_command_register_to_idle() {
    let mut d = Rc522::new("PA4");
    write_reg(&mut d, REG_COMMAND, CMD_SOFTRESET);
    assert_eq!(read_reg(&mut d, REG_COMMAND), CMD_IDLE);
}

#[test]
fn crc_a_matches_iso_preset_6363() {
    assert_eq!(crc_a(&[]), [0x63, 0x63]);
    assert_eq!(crc_a(&[0x50, 0x00]), [0x57, 0xCD]);
}

#[test]
fn calc_crc_command_fills_result_registers() {
    let mut d = Rc522::new("PA4");
    let got = calc_crc(&mut d, &[0x50, 0x00]);
    assert_eq!(got, crc_a(&[0x50, 0x00]));
}

#[test]
fn reqa_times_out_when_no_card_in_the_field() {
    let mut d = Rc522::new("PA4");
    let (irq, data) = transceive(&mut d, &[PICC_REQA], 7);
    assert_ne!(irq & COMIRQ_TIMER, 0, "no card → TimerIRq");
    assert_eq!(irq & COMIRQ_RX, 0, "no card → no RxIRq");
    assert!(data.is_empty());
}

#[test]
fn reqa_returns_atqa_when_card_present() {
    let mut d = Rc522::new("PA4");
    d.set_card(Some([0xDA, 0x01, 0x7A, 0x01]));
    let (irq, data) = transceive(&mut d, &[PICC_REQA], 7);
    assert_ne!(irq & COMIRQ_RX, 0);
    assert_eq!(data, ATQA);
}

#[test]
fn wupa_wakes_a_halted_card() {
    let mut d = Rc522::new("PA4");
    d.set_card(Some([0x11, 0x22, 0x33, 0x44]));
    // Drive to Active then Halt.
    let _ = transceive(&mut d, &[PICC_REQA], 7);
    let bcc = 0x11 ^ 0x22 ^ 0x33 ^ 0x44;
    let mut sel = vec![PICC_SEL_CL1, 0x70, 0x11, 0x22, 0x33, 0x44, bcc];
    let crc = crc_a(&sel);
    sel.extend_from_slice(&crc);
    let _ = transceive(&mut d, &sel, 0);
    let mut hlta = vec![PICC_HLTA, 0x00];
    let hcrc = crc_a(&hlta);
    hlta.extend_from_slice(&hcrc);
    let (hirq, _) = transceive(&mut d, &hlta, 0);
    assert_ne!(hirq & COMIRQ_TIMER, 0, "HLTA succeeds as timeout");

    // REQA must ignore Halt; WUPA must wake.
    let (rirq, _) = transceive(&mut d, &[PICC_REQA], 7);
    assert_ne!(rirq & COMIRQ_TIMER, 0);
    let (wirq, wdata) = transceive(&mut d, &[PICC_WUPA], 7);
    assert_ne!(wirq & COMIRQ_RX, 0);
    assert_eq!(wdata, ATQA);
}

#[test]
fn anticollision_and_select_return_uid_bcc_and_sak() {
    let uid = [0xDA, 0x01, 0x7A, 0x01];
    let mut d = Rc522::new("PA4");
    d.set_card(Some(uid));
    let (_irq, atqa) = transceive(&mut d, &[PICC_REQA], 7);
    assert_eq!(atqa, ATQA);

    let (airq, anti) = transceive(&mut d, &[PICC_SEL_CL1, 0x20], 0);
    assert_ne!(airq & COMIRQ_RX, 0);
    assert_eq!(&anti[..4], &uid);
    assert_eq!(anti[4], uid[0] ^ uid[1] ^ uid[2] ^ uid[3]);

    let mut sel = vec![PICC_SEL_CL1, 0x70];
    sel.extend_from_slice(&uid);
    sel.push(anti[4]);
    let crc = crc_a(&sel);
    sel.extend_from_slice(&crc);
    let (sirq, sak) = transceive(&mut d, &sel, 0);
    assert_ne!(sirq & COMIRQ_RX, 0);
    assert_eq!(sak[0], SAK);
    assert_eq!(&sak[1..], &crc_a(&[SAK]));
}

#[test]
fn set_input_places_and_removes_card() {
    let mut d = Rc522::new("PA4");
    assert!(!d.card_present());
    d.set_input("uid", 0x5102_7A02u32 as f64).unwrap();
    d.set_input("card_present", 1.0).unwrap();
    assert!(d.card_present());
    assert_eq!(d.uid(), [0x51, 0x02, 0x7A, 0x02]);
    let (_irq, data) = transceive(&mut d, &[PICC_REQA], 7);
    assert_eq!(data, ATQA);

    d.set_input("card_present", 0.0).unwrap();
    assert!(!d.card_present());
    let (irq, data) = transceive(&mut d, &[PICC_REQA], 7);
    assert_ne!(irq & COMIRQ_TIMER, 0);
    assert!(data.is_empty());
}

#[test]
fn unknown_input_channel_is_rejected() {
    let mut d = Rc522::new("PA4");
    assert!(d.set_input("pressure", 1.0).is_err());
}

#[test]
fn shell_golden_transcript_matches_migration_parity() {
    // Same conversation as spi_shell_migration_parity::rc_script / RC522_GOLDEN.
    let mut d = Rc522::new("PA4");
    let mut got = Vec::new();
    let script: &[&[u8]] = &[
        &[0x80 | (0x37 << 1), 0x00],
        &[0x01 << 1, 0x0F],
        &[0x80 | (0x01 << 1), 0x00],
        &[0x01 << 1, 0x03],
        &[0x80 | (0x01 << 1), 0x00],
        &[0x11 << 1, 0x3D],
        &[0x80 | (0x11 << 1), 0x00],
        &[0x80 | (0x0A << 1), 0x00],
        &[0x80 | (0x3F << 1), 0x00],
    ];
    for frame in script {
        got.extend(xfer(&mut d, frame));
    }
    const GOLDEN: &[u8] = &[
        0x00, 0x92, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3D, 0x00,
        0x00, 0x00, 0x00,
    ];
    assert_eq!(got, GOLDEN);
}
