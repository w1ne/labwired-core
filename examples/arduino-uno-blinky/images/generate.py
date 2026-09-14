#!/usr/bin/env python3
"""Compose the Arduino Uno R3 pinout around the board drawing.

    python3 examples/arduino-uno-blinky/images/generate.py          # regenerate
    python3 examples/arduino-uno-blinky/images/generate.py --check  # verify

board.svg is NOT drawn here. It is exported from the Playground's canvas renderer
(labwired: tools/boards/arduino-uno/export-art.mjs), whose layout is generated
from Arduino's own board file UNO-TH_Rev3e.brd (CC BY-SA 4.0), so the docs, the
pinout and the canvas show the same board. This script embeds that drawing and
places a tag stack on every header pin: Arduino number, AVR port bit and
alternate functions from ArduinoCore-avr variants/standard/pins_arduino.h and
the ATmega328P datasheet. Header positions are the R3 shield drawing in inches
from the lower-left corner (rows at 0.1 in and 2.0 in, the 0.16 in D7-D8 gap).
"""
from pathlib import Path

HERE = Path(__file__).resolve().parent
FONT = "ui-sans-serif,system-ui,sans-serif"

# (x_in, silk, arduino, port, functions) left to right. functions: (label, kind)
TOP = [
    (0.74, "", "SCL", "PC5", [("I2C SCL", "comm")]),
    (0.84, "", "SDA", "PC4", [("I2C SDA", "comm")]),
    (0.94, "AREF", "AREF", None, [("ADC ref", "analog")]),
    (1.04, "GND", "GND", None, []),
    (1.14, "13", "D13", "PB5", [("SPI SCK", "comm"), ("LED", "led")]),
    (1.24, "12", "D12", "PB4", [("SPI MISO", "comm")]),
    (1.34, "~11", "D11", "PB3", [("PWM", "pwm"), ("SPI MOSI", "comm")]),
    (1.44, "~10", "D10", "PB2", [("PWM", "pwm"), ("SPI SS", "comm")]),
    (1.54, "~9", "D9", "PB1", [("PWM", "pwm")]),
    (1.64, "8", "D8", "PB0", []),
    (1.80, "7", "D7", "PD7", []),
    (1.90, "~6", "D6", "PD6", [("PWM", "pwm")]),
    (2.00, "~5", "D5", "PD5", [("PWM", "pwm")]),
    (2.10, "4", "D4", "PD4", []),
    (2.20, "~3", "D3", "PD3", [("PWM", "pwm"), ("INT1", "irq")]),
    (2.30, "2", "D2", "PD2", [("INT0", "irq")]),
    (2.40, "TX→1", "D1", "PD1", [("UART TX", "comm")]),
    (2.50, "RX←0", "D0", "PD0", [("UART RX", "comm")]),
]
BOTTOM = [
    (1.10, "", "NC", None, []),
    (1.20, "IOREF", "IOREF", None, []),
    (1.30, "RESET", "RESET", None, []),
    (1.40, "3.3V", "3.3V", None, []),
    (1.50, "5V", "5V", None, []),
    (1.60, "GND", "GND", None, []),
    (1.70, "GND", "GND", None, []),
    (1.80, "Vin", "VIN", None, []),
    (2.00, "A0", "A0", "PC0", [("ADC0", "analog")]),
    (2.10, "A1", "A1", "PC1", [("ADC1", "analog")]),
    (2.20, "A2", "A2", "PC2", [("ADC2", "analog")]),
    (2.30, "A3", "A3", "PC3", [("ADC3", "analog")]),
    (2.40, "A4", "A4", "PC4", [("ADC4", "analog"), ("I2C SDA", "comm")]),
    (2.50, "A5", "A5", "PC5", [("ADC5", "analog"), ("I2C SCL", "comm")]),
]

KIND = {
    "pin": ("#1f2937", "#ffffff"),
    "port": ("#e5e7eb", "#111827"),
    "pwm": ("#f59e0b", "#111827"),
    "comm": ("#0d9488", "#ffffff"),
    "analog": ("#16a34a", "#ffffff"),
    "power": ("#dc2626", "#ffffff"),
    "ground": ("#111827", "#ffffff"),
    "irq": ("#7c3aed", "#ffffff"),
    "led": ("#facc15", "#111827"),
    "other": ("#9ca3af", "#111827"),
}


def fmt(v):
    return f"{v:.1f}".rstrip("0").rstrip(".")


class Board:
    """Maps board inches to SVG pixels for one placement."""

    def __init__(self, scale, x0, y0):
        self.s, self.x0, self.y0 = scale, x0, y0

    def x(self, inch):
        return self.x0 + inch * self.s

    def y(self, inch_from_bottom):
        return self.y0 + (2.1 - inch_from_bottom) * self.s


def tag_stack(x, y, tags, direction):
    """Tags along a rotated axis from (x, y). direction=+1 up, -1 down."""
    out, cursor = [], 0.0
    parts = [f'<g transform="translate({fmt(x)} {fmt(y)}) rotate(-90)">']
    for label, kind in tags:
        fill, ink = KIND[kind]
        w = len(label) * 6.3 + 10
        start = cursor if direction > 0 else -cursor - w
        stroke = ' stroke="#9ca3af" stroke-width="0.75"' if kind == "port" else ""
        parts.append(f'<rect x="{fmt(start)}" y="-8" width="{fmt(w)}" height="16" rx="4" fill="{fill}"{stroke}/>')
        parts.append(f'<text x="{fmt(start + w / 2)}" y="3.8" text-anchor="middle" font-family="{FONT}" font-size="10.5" font-weight="600" fill="{ink}">{label}</text>')
        cursor += w + 3
    parts.append("</g>")
    out.append("".join(parts))
    return out, cursor


def pin_tags(arduino, port, fns):
    kind = "pin"
    if arduino in ("GND",):
        kind = "ground"
    elif arduino in ("5V", "3.3V", "VIN", "IOREF"):
        kind = "power"
    elif arduino in ("NC", "RESET"):
        kind = "other"
    tags = [(arduino, kind)]
    if port:
        tags.append((port, "port"))
    return tags + list(fns)


# board.svg is in canvas units, 140 per inch, with the PCB's left edge at x=36
# (the USB-B shell overhangs it). These must match arduino-uno-layout.generated.ts.
CANVAS_PER_INCH = 140
CANVAS_BOARD_LEFT = 36
CANVAS_W, CANVAS_H = 414, 294


def embedded_board(b):
    """board.svg as a nested <svg>, scaled so its PCB lands on Board `b`."""
    src = (HERE / "board.svg").read_text(encoding="utf-8")
    inner = src[src.index(">", src.index("<svg")) + 1:src.rindex("</svg>")]
    k = b.s / CANVAS_PER_INCH
    return (f'<svg x="{fmt(b.x0 - CANVAS_BOARD_LEFT * k)}" y="{fmt(b.y0)}" width="{fmt(CANVAS_W * k)}" '
            f'height="{fmt(CANVAS_H * k)}" viewBox="0 0 {CANVAS_W} {CANVAS_H}">{inner}</svg>')


def pinout_svg():
    S, X0, Y0 = 220, 210, 250
    b = Board(S, X0, Y0)
    W, H = 1010, 1030
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">',
           '<title>UNO R3 pinout</title>',
           '<desc>Header pins of the ATmega328P UNO R3 with Arduino pin numbers, AVR port bits and alternate functions.</desc>',
           f'<rect width="{W}" height="{H}" fill="#FFFFFF"/>',
           f'<text x="{W / 2}" y="44" text-anchor="middle" font-family="{FONT}" font-size="26" font-weight="700" fill="#111827">UNO R3 pinout</text>',
           f'<text x="{W / 2}" y="70" text-anchor="middle" font-family="{FONT}" font-size="14" fill="#4b5563">ATmega328P · 5 V logic · 16 MHz · LED_BUILTIN = D13 (PB5)</text>',
           embedded_board(b)]
    top_anchor = b.y(2.1) - 10
    for x_in, _silk, arduino, port, fns in TOP:
        px = b.x(x_in)
        out.append(f'<line x1="{fmt(px)}" y1="{fmt(b.y(2.0))}" x2="{fmt(px)}" y2="{fmt(top_anchor)}" stroke="#6b7280" stroke-width="1"/>')
        tags, _ = tag_stack(px, top_anchor - 4, pin_tags(arduino, port, fns), +1)
        out += tags
    bottom_anchor = b.y(0) + 10
    for x_in, _silk, arduino, port, fns in BOTTOM:
        px = b.x(x_in)
        out.append(f'<line x1="{fmt(px)}" y1="{fmt(b.y(0.1))}" x2="{fmt(px)}" y2="{fmt(bottom_anchor)}" stroke="#6b7280" stroke-width="1"/>')
        tags, _ = tag_stack(px, bottom_anchor + 4, pin_tags(arduino, port, fns), -1)
        out += tags

    def callout(x_in, y_in, lx, ly, text, anchor):
        out.append(f'<line x1="{fmt(b.x(x_in))}" y1="{fmt(b.y(y_in))}" x2="{fmt(lx)}" y2="{fmt(ly)}" stroke="#6b7280" stroke-width="1"/>')
        for i, line in enumerate(text.split("\n")):
            out.append(f'<text x="{fmt(lx + (-6 if anchor == "end" else 6))}" y="{fmt(ly + 4 + i * 16)}" text-anchor="{anchor}" font-family="{FONT}" font-size="13" fill="#111827">{line}</text>')

    callout(0.225, 1.94, 150, b.y(2.2), "RESET button", "end")
    callout(0.05, 1.55, 150, b.y(1.55), "USB-B\n16U2 USB-serial\n= Serial (D0/D1)", "end")
    callout(0.79, 1.3, 150, b.y(1.0), "ATmega16U2", "end")
    callout(0.2, 0.345, 150, b.y(0.345), "DC jack\n7-12 V", "end")
    callout(2.6, 1.1, 860, b.y(1.1), "ICSP\n(ATmega328P)", "start")
    callout(2.3, 0.67, 860, b.y(0.67), "ATmega328P-PU\nDIP-28", "start")
    callout(1.085, 1.625, 860, b.y(1.85), "L = D13", "start")

    legend = (("Arduino pin", "pin"), ("AVR port bit", "port"), ("PWM (analogWrite)", "pwm"), ("UART / SPI / I2C", "comm"),
              ("Analog input", "analog"), ("External interrupt", "irq"), ("Power", "power"), ("Ground", "ground"))
    for i, (label, kind) in enumerate(legend):
        fill, _ink = KIND[kind]
        stroke = ' stroke="#9ca3af"' if kind == "port" else ""
        x, ly = 40 + (i % 4) * 235, 925 + (i // 4) * 26
        out.append(f'<rect x="{x}" y="{ly - 11}" width="16" height="16" rx="3" fill="{fill}"{stroke}/>')
        out.append(f'<text x="{x + 22}" y="{ly + 2}" font-family="{FONT}" font-size="13" fill="#111827">{label}</text>')
    out.append(f'<text x="40" y="990" font-family="{FONT}" font-size="12.5" fill="#4b5563">SCL/SDA on the top header are the same copper as A5/A4. The DIP-28 part has no ADC6/ADC7.</text>')
    out.append(f'<text x="40" y="1010" font-family="{FONT}" font-size="12.5" fill="#4b5563">Sources: ArduinoCore-avr variants/standard/pins_arduino.h, ATmega328P datasheet, Arduino A000066 datasheet and schematic.</text>')
    out.append("</svg>")
    return "\n".join(out) + "\n"


# The docs site builds from docs/, which cannot reach examples/, so the board
# page embeds byte-identical copies from docs/assets. `--check` fails when the
# pinout is stale or any copy differs.
DOCS_ASSETS = HERE.parents[2] / "docs" / "assets" / "boards" / "arduino-uno"

if __name__ == "__main__":
    import sys

    pinout = pinout_svg()
    expected = {
        HERE / "pinout.svg": pinout,
        DOCS_ASSETS / "pinout.svg": pinout,
        DOCS_ASSETS / "board.svg": (HERE / "board.svg").read_text(encoding="utf-8"),
    }
    if "--check" in sys.argv:
        stale = [str(t) for t, text in expected.items() if not t.exists() or t.read_text(encoding="utf-8") != text]
        if stale:
            sys.exit("stale: " + ", ".join(stale) + "\nrun: python3 examples/arduino-uno-blinky/images/generate.py")
        print("uno images up to date")
        sys.exit(0)
    DOCS_ASSETS.mkdir(parents=True, exist_ok=True)
    for target, text in expected.items():
        target.write_text(text, encoding="utf-8")
        print(f"wrote {target} ({target.stat().st_size} bytes)")
