# nRF5340 Zephyr boot fixture

The simulator boots **unmodified upstream Zephyr** on the Nordic nRF5340
application core (Cortex-M33). This directory holds the recipe that produces the
committed fixture:

    tests/fixtures/nrf5340-zephyr-hello.elf

It is the stock `samples/hello_world` from Zephyr v3.7.x, built for board target
`nrf5340dk/nrf5340/cpuapp`. Nothing in Zephyr is patched — the point of the
profile is that community/vendor firmware runs as-is and verifiably prints its
banner over the UARTE0 console.

## Rebuild

```sh
# Needs a Zephyr v3.7 west workspace (default ~/zephyrproject) and
# arm-none-eabi-gcc on PATH (gnuarmemb variant — no Zephyr SDK required).
./build.sh
```

`build.sh` rebuilds and republishes the fixture. `build/` is gitignored; the
committed ELF is the source of truth.

## What proves it works

- `firmware_survival::test_nrf5340_zephyr_survival` boots this ELF end to end and
  asserts the console emits `Hello World! nrf5340dk/nrf5340/cpuapp`.
- `tests/nrf5340_clock_boot.rs` is the ELF-independent twin: it replays the
  CLOCK HFCLK/LFCLK start→started poll loops and the non-secure peripheral-alias
  mapping at the bus level.

The boot reaches `main` because the shared Nordic CLOCK / UARTE EasyDMA / RTC
behavioural models settle the status the Zephyr drivers spin on, and the chip
profile maps every region the firmware touches at the `0x5000_0000` non-secure
alias (verified violation-free with `LABWIRED_TRACE_VIOLATIONS=1`).
