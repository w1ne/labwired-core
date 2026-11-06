# Hosted verification result

Checked 2026-11-06 UTC with `labwired_verify` on the S3 DevKitC-1 N16R8 board
diagram, with a NeoPixel attached to GPIO48.

- The UART oracle passed: firmware emitted `ESP32_S3_DEVKITC1_V10_RMT_OK`
  after observing RMT channel 0 TX_END.
- The LED display oracle failed. The hosted model decoded one pixel but
  reported bytes `[0, 0, 0]` (`lit_pixels: 0`).
- A separate hosted probe using `adafruit/Adafruit NeoPixel` emitted its serial
  marker but decoded no GPIO48 pixel.
- The verifier also reported its usual ideal-VDD approximation; this does not
  explain the black pixel, which is the direct reason the LED clause failed.
- The local repository smoke passes and reports a non-black decoded pixel.

The local simulation and hosted verifier therefore disagree about the GPIO48
WS2812 output, and the hosted Adafruit NeoPixel path also remains unsupported
by this evidence. This board revision is covered by the passing local smoke,
but the hosted LED output is not claimed as model-verified. The structured
result is in [`hosted-verification.json`](hosted-verification.json); local
build and smoke output is in this directory's `logs/` and
`simulation-output/` folders.
