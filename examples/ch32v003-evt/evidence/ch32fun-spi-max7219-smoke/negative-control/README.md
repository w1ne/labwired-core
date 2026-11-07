# Negative control: 16-bit SPI frame reaches the device as its low byte only

`low-byte-only.log` is the same smoke, same firmware, with the SPI byte-level
path temporarily put back to its old behaviour (a CR1.DFF frame hands the
attached device `mosi & 0xFF` and nothing else). Result: 5/10.

Every MAX7219 write is `reg << 8 | data`, so the device saw only data bytes and
paired them up as its own (address, data) frames. The panel ended with
display test on (all 64 lit), shutdown still set, intensity 0 and scan limit
0 - none of the example's init writes landed where it sent them.

With the fix (high byte first, then low byte): 10/10, intensity 15, scan
limit 7, shutdown clear, 12 of 64 LEDs lit as Code-B 7..0 (max7219-state.json).
