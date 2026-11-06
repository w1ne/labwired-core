# External component — M5Stack AtomS3-Lite

The board system connects a one-pixel `neopixel` model to GPIO35, the onboard
WS2812C-2020 data input. The smoke sends the GRB frame by toggling GPIO35 and
checks the decoded pixel color. The model covers digital pulse timing and
protocol decoding; it does not model LED current, brightness, voltage
thresholds, or optical output.
