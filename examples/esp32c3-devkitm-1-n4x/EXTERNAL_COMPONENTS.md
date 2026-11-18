# External components

The system profile attaches a one-pixel WS2812/NeoPixel observer to GPIO8,
which Espressif's ESP32-C3-DevKitM-1 guide identifies as the onboard addressable
RGB LED data pin. The observer decodes the signal emitted by firmware; it does
not model LED current, supply voltage, package behavior, or optical output.
