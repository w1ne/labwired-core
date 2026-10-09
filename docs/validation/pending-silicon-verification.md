# Pending silicon verification

Model changes that are consistent with the TRM and the simulator's own tests
but have not yet been checked on real hardware. Each entry stays open until a
capture from silicon confirms or corrects it.

## ESP32-C3 / ESP32-S3: GPIO_IN reads back an input-enabled output pad

- Change: `GPIO_IN` (C3, S3 bank 0) and `GPIO_IN1` (S3 bank 1) return the pad's
  own driven level when the output driver is on (`GPIO_ENABLE`) and IO_MUX
  `FUN_IE` (bit 9) is set. Before, they ignored the output driver, so Arduino
  `digitalWrite(4, HIGH); digitalRead(4)` returned 0. The ESP32-C6 shares the
  C3 GPIO model and gets the same behavior.
- Field report: hosted-MCP blink lab, share `59iocidFNefH`, printed
  `LED ON GPIO4=0` every cycle; after the fix it prints `GPIO4=1` / `GPIO4=0`.
- Hardware recipe (ESP32-C3; board availability unknown, nothing confirmed on
  the bench yet. The S3 bench board can run the same sketch):
  1. Flash the sketch from share `59iocidFNefH` (Arduino-ESP32 3.x,
     `pinMode(4, OUTPUT)`, then `digitalWrite` + `digitalRead` on GPIO4) to an
     ESP32-C3 SuperMini or DevKitM-1, with GPIO4 -> 220 R -> LED -> GND.
  2. Capture UART0 at 115200. Expect `LED ON GPIO4=1` and `LED OFF GPIO4=0`.
  3. Clear `FUN_IE`: `REG_CLR_BIT(IO_MUX_GPIO4_REG, FUN_IE)` after `pinMode`.
     Expect `GPIO4=0` in both states (input buffer off).
  4. Also print `REG_READ(GPIO_IN_REG)`, `GPIO_ENABLE_REG` and
     `IO_MUX_GPIO4_REG` (expect `0x1a02` with FUN_IE set) and diff them against
     the simulator's register values for the same sketch.
- Oracle tests: `esp32c3_gpio_in_reads_back_an_input_enabled_output_pad`,
  `esp32s3_gpio_in_reads_back_an_input_enabled_output_pad`.
