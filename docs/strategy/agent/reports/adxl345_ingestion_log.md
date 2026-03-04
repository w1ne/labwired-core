# ADXL345 Ingestion Log

This document tracks the end-to-end ingestion and verification process for the ADXL345 3-axis accelerometer.

## 📁 Source Information
- **Device**: Analog Devices ADXL345 (Accelerometer)
- **Datasheet**: [ADXL345.pdf](file:///home/andrii/Projects/labwired/ai/tests/fixtures/adxl345.pdf)
- **Page Range**: 10-30 (Register definitions and behavior)

## 🛠️ Ingestion Pipeline Status

### Stage 1: Register Discovery [x]
- Target: Identifying memory-mapped registers (0x00 to 0x39).
- Found: `DEVID`, `THRESH_TAP`, `OFSX`, `OFSY`, `OFSZ`, `DUR`, `Latent`, `Window`, `THRESH_ACT`, `THRESH_INACT`, `TIME_INACT`, `ACT_INACT_CTL`, `THRESH_FF`, `TIME_FF`, `TAP_AXES`, `ACT_TAP_STATUS`, `BW_RATE`, `POWER_CTL`, `INT_ENABLE`, `INT_MAP`, `INT_SOURCE`, `DATA_FORMAT`, `DATAX0`, `DATAX1`, `DATAY0`, `DATAY1`, `DATAZ0`, `DATAZ1`, `FIFO_CTL`, `FIFO_STATUS`

### Stage 2: Bitfield Extraction [x]
- Target: Mapping features like Tap detection, Activity/Inactivity, and FIFO control.
- Status: Completed for 30 registers.

### Stage 3: Behavioral Synthesis [x]
- Target: Capturing causal links (e.g., "Activity on X-axis triggers bit 4 in INT_SOURCE").
- Result: Synthesized complex event logic for Tap, Activity, and Interrupts.

## 📊 Evaluation & Fidelity Check
- [x] Schema Validity: Verified.
- [ ] Register Offset Accuracy: Manual check in progress...
- [ ] Side-effect Logic Accuracy: Manual check in progress...
- [x] Schema Validity: Verified.
- [x] Register Offset Accuracy: Verified (Matches Analog Devices spec).
- [x] Side-effect Logic Accuracy: Verified (Captures complex ADXL345 behaviors).
- [x] Codegen Success (Rust): Drivers generated and verified.

## 🚀 Codegen Verification
- Drivers generated to: `ai/tests/adxl345_driver.rs` and `ai/tests/lm75b_driver.rs`
- Validated: `THRESH_TAP`, `BW_RATE`, `INT_SOURCE` bitfields are correctly mapped and accessible.


## 📊 Evaluation & Fidelity Check
(To be populated after ingestion)
- [ ] Schema Validity
- [ ] Register Offset Accuracy
- [ ] Side-effect Logic Accuracy
- [ ] Codegen Success (Rust)
