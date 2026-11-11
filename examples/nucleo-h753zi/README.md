# NUCLEO-H753ZI quick start

## Demand rank

This board covers the professional STM32 segment. The demand list ranks other
STM32 models fourth at **13%** and reports Nucleo/Discovery kits in **23%** of
projects. ST's live stock and product status are recorded in
[vendor-status.md](vendor-status.md).

## Copy-paste example

This bare-metal Rust example enables GPIOB and toggles the board's default
LD1 pin (PB0):

```rust
const RCC_AHB4ENR: *mut u32 = 0x5802_44E0 as *mut u32;
const GPIOB_MODER: *mut u32 = 0x5802_0400 as *mut u32;
const GPIOB_BSRR: *mut u32 = 0x5802_0418 as *mut u32;

unsafe {
    core::ptr::write_volatile(RCC_AHB4ENR, 1 << 1);
    let moder = core::ptr::read_volatile(GPIOB_MODER);
    core::ptr::write_volatile(GPIOB_MODER, (moder & !0b11) | 0b01);
    loop {
        core::ptr::write_volatile(GPIOB_BSRR, 1); // LD1 on
        for _ in 0..200_000 { cortex_m::asm::nop(); }
        core::ptr::write_volatile(GPIOB_BSRR, 1 << 16); // LD1 off
        for _ in 0..200_000 { cortex_m::asm::nop(); }
    }
}
```

The complete smoke also routes PD8 to USART3 AF7 and writes
`H753_USART3_OK` to the default ST-LINK VCP. Build and run it from the repo
root:

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-h753zi \
  --crate firmware-nucleo-h753zi-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-h753zi/io-smoke.yaml \
  --system configs/systems/nucleo-h753zi.yaml \
  --out-dir examples/nucleo-h753zi/evidence
```

The smoke requires the UART marker, at least eight PB0 edges, and the bounded
stop condition. The descriptor models the H7 RCC, GPIOB/C/D, USART3, SysTick,
the smoke memory regions, LD1, and B1. It does not model the ST-LINK bridge,
other user LEDs, Ethernet, USB, or other board peripherals. Simulator output
does not prove the physical pins or a physical board.

See [validation notes](VALIDATION.md) and [live vendor status](vendor-status.md).
