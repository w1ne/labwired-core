# External components

The simulator smoke needs no external component. The actual DevKitC-1 board has
an addressable RGB LED on GPIO27, which is intentionally outside this model.
For a physical GPIO5 check, connect an external LED and current-limiting resistor
between J3 pin 9 (GPIO5) and ground.
