# Capture provenance

`j1939-slice-8s.log` is an 8-second slice (candump format) of a J1939 sample
capture provided by CSS Electronics (Martin Falch, 2026-07-01) from their
CANsub webCAN CSV export, converted with `webcan2candump.py` from the
cansub-replay-ci template. Redistributed with permission.

The slice deliberately contains a concurrently interleaved pair of J1939 BAM
broadcasts (Engine Configuration from SA 0x00 and Retarder Configuration from
SA 0x0F) plus DM1 traffic from 9 source addresses.
