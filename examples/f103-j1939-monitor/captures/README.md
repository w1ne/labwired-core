# Capture provenance

`synthetic-j1939-8s.log` is synthesized by `tools/gen_synthetic_j1939.py`. It
is deterministic and regenerable, and contains no third-party data.

Regenerate it (byte-identical output) with:
```
python3 tools/gen_synthetic_j1939.py captures/synthetic-j1939-8s.log
```

The generated traffic (1785 frames over 8 s) contains:
- engine-speed "noise" frames (PGN 0xF004 every 5 ms, PGN 0xFEF1 every 100 ms)
- DM1 active-diagnostic-trouble-code broadcasts from 9 source addresses
- 3 concurrent pairs of interleaved J1939 BAM (multi-packet transport
  protocol) sessions: Engine Configuration (PGN 0xFEE3, SA 0x00) and Retarder
  Configuration (PGN 0xFEE1, SA 0x0F), opening ~10 ms apart with their TP.DT
  frames alternating before either session closes
