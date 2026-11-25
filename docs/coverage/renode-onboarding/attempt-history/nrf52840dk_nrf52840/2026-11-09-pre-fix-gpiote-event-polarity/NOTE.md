# nRF52840 DK — GPIOTE Event-mode HiToLo missed the first press (pre-fix)

Run: `labwired test --script examples/renode-onboarding/nrf52840dk_nrf52840-smoke.yaml`
on the new 49-peripheral DK row, CLI built before the GPIOTE fix (target/release/labwired, 8 Nov).

Result: FAIL 3/5. Every block passes except `gpiote in1 p0.11 ... fall=0 pend=0`
(summary 0x3FF, bit 10 missing).

Cause: `Nrf52Gpiote::observe_gpio_change` decided an edge by comparing the
reported level with `channel_in_level[ch]`, which starts at 0 and is never
seeded from the pad. Button 1 idles HIGH on its pull-up, so the press is
reported as `level 0` against a remembered 0 and the HiToLo edge is dropped
(the release then looks like LoToHi). The bus only reports pins whose IN level
actually changed, so the new level alone gives the edge direction; the fix
decides from that.
