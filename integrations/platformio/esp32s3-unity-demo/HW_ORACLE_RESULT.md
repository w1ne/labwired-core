# HW oracle — real ESP32-S3 ground truth (2026-06-02)

Captured from the physical ESP32-S3 over USB-JTAG (no buttons, fully remote),
reading the Unity result struct from RAM. This is the reference the simulator
must reproduce once its Xtensa scheduler lands.

## Procedure (reproducible)
```
# 1. flash the no-CDC firmware AND boot it with a full RTC reset (buttonless)
esptool --port /dev/ttyACM0 --chip esp32s3 --before default-reset \
        --after watchdog-reset write-flash 0x0 \
        .pio/build/esp32-s3-devkitc-1/firmware.factory.bin
sleep 4   # app boot + delay(2000) + Unity

# 2. JTAG halt + read the Unity struct (offsets account for CurrentDetail1/2)
openocd -f board/esp32s3-builtin.cfg -c "init" -c "halt" \
        -c "mdw 0x3fc9a788 10" -c "resume" -c "exit"
```

## Result (raw)
```
0x3fc9a788: 3c030199 3c0301c6 00000000 00000000 00000010 00000002 00000000 00000000
TestFile        @0x3c030199 = "test/test_basic/test_basic.cpp"
CurrentTestName @0x3c0301c6 = "test_string"
```

## Decoded (UNITY_STORAGE_T, with CurrentDetail1/2 present)
| field | offset | value |
|---|---|---|
| TestFile (ptr) | +0 | →"test/test_basic/test_basic.cpp" |
| CurrentTestName (ptr) | +4 | →"test_string" |
| CurrentDetail1 / 2 | +8/+12 | NULL |
| CurrentTestLineNumber | +16 | 16 |
| **NumberOfTests** | **+20** | **2** |
| **TestFailures** | **+24** | **0** |
| **TestIgnores** | **+28** | **0** |

**Verdict: 2 Tests, 0 Failures, 0 Ignored — PASS on real silicon.**

The closed loop is proven end-to-end on hardware: agent builds firmware in
PlatformIO → flash → buttonless boot → JTAG reads the verified result. The sim
target is to emit the same `2 Tests 0 Failures` once its scheduler reaches
`setup()`.

---

## Model-vs-silicon verification: SALT / SALTU (2026-06-02)

The instructions added to the sim this session were verified directly against
the real ESP32-S3 by injecting operands and single-stepping the real `saltu`/
`salt` in the firmware's IRAM over JTAG, then comparing to the sim's semantics.

| # | instr | inputs | silicon result | sim semantics | match |
|---|-------|--------|----------------|---------------|-------|
| 1 | SALTU | 5 <ᵤ 10            | 1 | (5<10)            | ✓ |
| 2 | SALTU | 10 <ᵤ 5            | 0 | 0                 | ✓ |
| 3 | SALTU | 0xFFFFFFFF <ᵤ 1    | 0 | unsigned ⇒ 0      | ✓ |
| 4 | SALT  | −1 <ₛ 1            | 1 | signed ⇒ 1        | ✓ |
| 5 | SALT  | 1 <ₛ −1            | 0 | signed ⇒ 0        | ✓ |

Vectors 3–5 confirm the sim got the signed-vs-unsigned distinction identical to
silicon. Sim impl: `crates/core/.../cpu/xtensa_lx7.rs` (SALT = `(AR[s] as i32) <
(AR[t] as i32)`, SALTU = `AR[s] < AR[t]` unsigned); decode unit-tested in
`decoder/xtensa.rs::salt_saltu_tests`. **SALT/SALTU model: silicon-verified.**

### Broader parity status (honest)
- ✅ CPU/ISA: the one instruction added this session matches silicon exactly.
- ⏳ Full behavioral parity (sim serial == HW serial) is pending the Xtensa
  scheduler — the sim doesn't reach `setup()`/Unity yet. The HW target is the
  `2 Tests, 0 Failures` captured above.
- ℹ️ The sim intentionally diverges from silicon during boot (it thunks ROM /
  handshake / cache), so a full-state boot diff is not a like-for-like compare;
  per-instruction CPU fidelity is the meaningful axis, and it checks out here.
