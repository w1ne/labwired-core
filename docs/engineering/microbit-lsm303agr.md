# micro:bit V2 LSM303AGR motion sensor

The micro:bit V2 carries an ST LSM303AGR on the internal I²C bus (TWIM0,
SCL P0.08 / SDA P0.16). It is two I²C slaves, and LabWired models each as a
declarative device:

| Component id    | Type              | Address | Descriptor |
|-----------------|-------------------|---------|------------|
| `accelerometer` | `lsm303agr_accel` | `0x19`  | [`lsm303agr_accel.yaml`](../../configs/devices/lsm303agr_accel.yaml) |
| `magnetometer`  | `lsm303agr_mag`   | `0x1E`  | [`lsm303agr_mag.yaml`](../../configs/devices/lsm303agr_mag.yaml) |

Both are attached in [`configs/systems/microbit-v2.yaml`](../../configs/systems/microbit-v2.yaml)
and [`examples/microbit-v2/system.yaml`](../../examples/microbit-v2/system.yaml).
The [Foundation I²C inventory](https://tech.microbit.org/hardware/i2c/) also
lists an FXOS8700 board variant; that variant is not selected here.

## Inputs

Each component takes live `x`, `y`, `z` stimulus: acceleration in g and
magnetic field in µT. They default to zero; no gravity or heading is invented.

## What is modelled

- Register maps, reset values and WHO_AM_I (`0x33` / `0x40`) per the
  [ST LSM303AGR datasheet, Rev 11](https://www.st.com/resource/en/datasheet/lsm303agr.pdf).
- Sub-address bit 7 (auto-increment) on both halves.
- Accelerometer output: left-justified 16-bit two's complement at the full
  scale `CTRL_REG4_A` FS[1:0] selects.
- Magnetometer output: 16-bit two's complement at 0.15 µT/LSB.

- Data-ready: each half raises its XYZ data-ready bits 10 ms (100 Hz) after
  it starts measuring or after its last output read, through the engine's
  `data_ready` rule. The accelerometer measures while `CTRL_REG1_A` ODR is
  non-zero; the magnetometer while `CFG_REG_A_M` MD[1] is clear (continuous or
  single). Polling firmware therefore reads about 100 samples a second, as on
  silicon, instead of one per poll.

## Not modelled

The configured ODR value (every rate paces at 100 Hz), a fixed sampling grid
(the period restarts from each output read), single-shot drop to idle,
overrun bits, block-data-update latching, resolution truncation of the low
bits, FIFO, interrupts (the shared P0.25 line is unconnected),
click/orientation detection, filters, self-test, temperature and hard-iron
offset subtraction. Each descriptor header lists these.

## Executable proof

`microbit_v2_motion_io_guest` assembles an original polled driver
([`motion-polled.inc`](../../examples/microbit-v2/motion-polled.inc)) with
`arm-none-eabi-gcc`, runs it on the simulated nRF52833, and drives both halves
through TWIM0 EasyDMA. Two held poses must produce exact raw samples:

```sh
cargo test -p labwired-core --features microbit-board-io-test --test microbit_v2_motion_io_guest
```
