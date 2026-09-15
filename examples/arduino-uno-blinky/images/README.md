# Arduino Uno R3 images

| File | What |
|------|------|
| `board.svg` | Top view of the board, with copper, pads, parts and silkscreen |
| `pinout.svg` | `board.svg` with a tag stack on every header pin: Arduino number, AVR port bit, alternate functions |
| `generate.py` | Composes `pinout.svg` and writes the `docs/assets` copies |

`board.svg` is exported from the Playground's canvas renderer
(labwired `tools/boards/arduino-uno/export-art.mjs`), so the docs, the pinout and
the canvas part show the same drawing. That renderer's layout is generated from
Arduino's own board file `UNO-TH_Rev3e.brd` (A000066 CAD files): outline,
mounting holes, traces, vias, pads and the position of every part. The
silkscreen and part appearance are matched to Arduino's A000066 photograph.

**Licence.** The board file is (c) Arduino S.r.l., CC BY-SA 4.0
(https://creativecommons.org/licenses/by-sa/4.0/). `board.svg` and `pinout.svg`
are derivatives and are shared under the same licence. No vendor logo or
wordmark is drawn.

Pin functions come from `ArduinoCore-avr` `variants/standard/pins_arduino.h`
and the ATmega328P datasheet.

```bash
python3 examples/arduino-uno-blinky/images/generate.py          # regenerate pinout + docs copies
python3 examples/arduino-uno-blinky/images/generate.py --check  # verify
```

The docs site builds from `docs/`, so the script also writes byte-identical
copies to `docs/assets/boards/arduino-uno/`, which the board page embeds.
