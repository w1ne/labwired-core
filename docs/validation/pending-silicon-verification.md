# Pending silicon verification

Model changes that are consistent with the TRM and the simulator's own tests
but have not yet been checked on real hardware. Each entry stays open until a
capture from silicon confirms or corrects it.

## STM32 legacy I2C (F1/F2/F4): master-receive pipeline with BTF and POS

- Change: a master receive now clocks bytes in only after ADDR is cleared
  (SR1 then SR2), one byte per nine SCL periods (from CCR), into DR (RXNE) and
  then the shift register (BTF), stretching while both are full. ACK is
  sampled when a byte completes, or one byte earlier with CR1.POS set. A NACKed
  byte ends the receive, and a STOP set while a byte is on the wire goes out
  after it. DR reads move the shift register into DR. Before, the model never
  raised BTF on a receive, so the HAL's last-two-bytes path (`I2C_MasterReceive_BTF`)
  never ran and every Wire read longer than one byte returned 0 bytes.
- Field report: MCP audit 2026-10-09, `Adafruit_BME280::begin()` failed on
  NUCLEO-F401RE and -F103RB; `requestFrom(0x76, 2)` after a register write
  returned 0.
- Hardware recipe (NUCLEO-F401RE and NUCLEO-F103RB, BME280 breakout at 0x76,
  SDA=D14/PB9, SCL=D15/PB8, 4.7 k pull-ups):
  1. Build and flash `tests/fixtures/stm32duino-bme280/main.ino` (`pio run -e
     f401` / `-e f103`). Capture USART2 (ST-LINK VCP) at 115200.
  2. Expect `reg 0x88 len 2 end=0 got=2`, `len 3 got=3`, `len 6 got=6`, the
     6-byte line starting with the 2- and 3-byte lines' bytes, `BME280 ok` and
     a temperature line. Compare the calibration bytes with the sim's.
  3. Logic analyzer on SDA/SCL: for the 2-byte read expect byte 1 ACKed and
     byte 2 NACKed, then STOP; about nine SCL periods between ADDR clear
     (SR2 read) and the first data byte.
  4. SWD during a polled 2-byte read (`HAL_I2C_Master_Receive`, POS=1, ACK=0
     before ADDR clear): break after ADDR clear and poll SR1; expect `0x0044`
     (RXNE|BTF) once both bytes are in, and DR then returning byte 1, byte 2.
- Oracle tests: `stm32duino_i2c_multibyte_bme280` (F401, F103), legacy unit
  tests `test_adxl345_devid_and_axis_read`,
  `test_i2c_single_byte_read_advances_device_once`, and
  `a_repeated_start_read_decodes_as_every_frame_that_crossed_the_bus`.

## STM32 modern I2C (L4/F7/H5/G0): NBYTES transfers and TC cleared by START

- Change: the phase model now moves CR2.NBYTES bytes per START (TXIS per byte
  on writes, RXNE per byte on reads, next byte after RXDR is read), then TC or
  the AUTOEND STOP. Setting START or STOP clears TC/TCR (RM0351 I2C_ISR). Before,
  it moved one byte, and TC left set by a SOFTEND register-pointer write made
  the repeated-START read fail with `HAL_I2C_ERROR_SIZE`.
- Field report: MCP audit 2026-10-09, `Adafruit_BME280::begin()` failed on
  NUCLEO-L476RG.
- Hardware recipe (NUCLEO-L476RG, BME280 at 0x76 on D14/D15):
  1. Flash the same sketch (`pio run -e l476`), capture USART2 at 115200, and
     expect the same lines as on the F401.
  2. SWD: after `Wire.endTransmission(false)` completes read `I2C1_ISR`
     (expect TC, `0x8041`); after the read's CR2 write with START, `ISR.TC`
     must read 0.
- Oracle test: `stm32duino_i2c_multibyte_bme280` (L476).

## ESP32-C3: mask ROM mirrors its boot log to UART0 and USB-Serial-JTAG

- Change: none in the peripheral models. Run paths that show both consoles in
  one stream (hosted C3 run with no declared console) now merge them through
  `ConsoleMerge`, which writes the bytes both consoles carry once. Before, the
  ROM banner appeared twice, interleaved mid-word.
- Field report: MCP audit 2026-10-09, `run-c3-accept` serial began
  `ESP-ROM:esp32c3-api1-20210207\r\nESP-ROBuild:Feb  7 2021...`.
- Hardware recipe (ESP32-C3 SuperMini or DevKitM-1, USB-UART adapter on
  GPIO21/GPIO20):
  1. Flash an Arduino sketch built with `ARDUINO_USB_CDC_ON_BOOT=0` that
     prints on `Serial0`.
  2. Open the USB CDC port and the adapter's port at 115200 at the same time,
     then press RESET.
  3. Expect the ROM banner and `load:`/`entry` lines on BOTH ports, byte for
     byte the same, and the sketch's output on UART0 only. If the CDC port
     shows less of the ROM log than UART0, the merge is still right (it keeps
     whatever both carry once) but `ConsoleCapture::unheard_output` docs need
     correcting.
- Oracle test: `c3_rom_banner_appears_once_in_the_merged_console` (CLI,
  hosted path) and the `ConsoleMerge` unit tests in `console.rs`.
