# RP2354B smoke (CLI proof)

CLI coverage cell for [`configs/chips/rp2354b.yaml`](../../configs/chips/rp2354b.yaml).
`ci/test.sh` runs the committed `tests/fixtures/rp2354b-smoke.elf` against
`configs/systems/rp2354b.yaml` and asserts both UART0 banners and sixteen
GPIO47 edges ([`io-smoke.yaml`](io-smoke.yaml)).

The firmware is [`firmware/smoke.c`](firmware/smoke.c): XOSC, PLL_SYS to
150 MHz, CLK_PERI, UART0 on GPIO0 at 1 Mbaud, then GPIO47 toggled through
SIO `GPIO_HI_OUT_XOR`. It includes only unmodified pico-sdk 2.3.1 register
headers. Rebuild it with:

```sh
PICO_SDK=/path/to/pico-sdk examples/rp2354b/firmware/build.sh
```

`build.sh` checks the SDK commit and that its tree is clean; the expected
ELF sha256 and the header hashes are in
[`firmware/source-and-build.json`](firmware/source-and-build.json).
