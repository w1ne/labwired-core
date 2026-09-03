# ydlidar_230400_22s.bin

A raw, unmodified capture of the RX line of a physical 360° scanning lidar.

| | |
|---|---|
| Captured | 2026-09-01 |
| Host | macOS, `termios` raw mode, no flow control |
| Port | USB serial via a WCH CH340 bridge (`1A86:7523`) |
| Link | 230400 8N1 |
| Duration | 22.16 s, continuous |
| Size | 306 098 bytes |
| Content | 3 794 frames, **0 rejected by checksum**; 89 331 ranging samples; 223 revolutions |

Nothing was written to the port — the device free-runs on power. The bytes are
exactly what came off the wire, in order, with no re-framing or filtering.

## Why it is in the repo

It is the oracle for `tests/ydlidar_silicon_parity.rs`. The test decodes every
frame, re-encodes it through the shipped codec in
`peripherals/components/ydlidar.rs`, and requires the result to be
byte-identical. That is what lets the model claim its wire format is faithful
rather than plausible.

Three properties of the format are only observable here, and a decoder-only
reading of the protocol misses all three:

- `FSA`/`LSA` bit 0 is a check bit and is **always 1**. `raw >> 1` hides it.
- Distances are quarter-millimetre: `raw & 3` takes all four values.
- Intensity is 6-bit stored `<< 2`; the low two bits were 0 in all 89 331 samples.

## Re-deriving the summary numbers

Frame walk: match `AA 55`, read `CT`, `LSN`, `FSA`, `LSA`, `CS`, then `3 * LSN`
sample bytes. Checksum is XOR-16 over `0x55AA`, `CT | LSN << 8`, `FSA`, each
sample's distance word and intensity byte, and finally `LSA`.
