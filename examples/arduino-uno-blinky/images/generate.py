#!/usr/bin/env python3
"""Generate the Arduino Uno R3 board illustration and pinout from one geometry.

    python3 examples/arduino-uno-blinky/images/generate.py

Writes board.svg and pinout.svg next to this file. Coordinates are the R3
shield drawing in inches from the lower-left corner (headers on the 0.1 in and
2.0 in rows, the 0.16 in D7-D8 gap, holes at (0.55, 0.1) (0.6, 2.0)
(2.6, 1.4) (2.6, 0.3), right edge stepped from 2.6 in to 2.7 in), cross-checked
against Arduino's A000066 front photograph. Pin functions come from
ArduinoCore-avr variants/standard/pins_arduino.h and the ATmega328P datasheet.

No vendor logo or wordmark is drawn: the silk shows the "UNO R3" model box
only. Flat style per the LabWired illustration standard (gold contacts, dark
packages, white ground).
"""
from pathlib import Path

HERE = Path(__file__).resolve().parent
FONT = "ui-sans-serif,system-ui,sans-serif"
PCB, EDGE, GOLD, PKG, SILK, METAL = "#0e7c86", "#0d1013", "#d4a84b", "#20252a", "#e8eee9", "#c9ccce"

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
HOLES = [(0.55, 0.1), (0.6, 2.0), (2.6, 1.4), (2.6, 0.3)]

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

    def rect(self, x_in, y_top_in, w_in, h_in, **attrs):
        """Rect by its top-left in (x, y-from-bottom of TOP edge) inches."""
        extra = " ".join(f'{k.replace("_", "-")}="{v}"' for k, v in attrs.items())
        return (f'<rect x="{fmt(self.x(x_in))}" y="{fmt(self.y(y_top_in))}" '
                f'width="{fmt(w_in * self.s)}" height="{fmt(h_in * self.s)}" {extra}/>')

    def draw(self, silk_labels=True):
        s, out = self.s, []
        X, Y = self.x, self.y
        # Outline with the stepped right edge.
        pts = [(0, 0), (2.6, 0), (2.6, 0.1), (2.7, 0.2), (2.7, 1.5), (2.6, 1.6), (2.6, 2.1), (0, 2.1)]
        d = "M" + " L".join(f"{fmt(X(a))} {fmt(Y(b))}" for a, b in pts) + " Z"
        out.append(f'<path d="{d}" fill="{PCB}" stroke="{EDGE}" stroke-width="{fmt(max(1.25, s / 90))}"/>')
        for hx, hy in HOLES:
            out.append(f'<circle cx="{fmt(X(hx))}" cy="{fmt(Y(hy))}" r="{fmt(0.055 * s)}" fill="#ffffff" stroke="{GOLD}" stroke-width="{fmt(0.022 * s)}"/>')
        # USB-B (overhangs the left edge) and the DC jack.
        out.append(self.rect(-0.25, 1.79, 0.61, 0.49, fill=METAL, stroke=EDGE, stroke_width="1.25", rx=fmt(0.02 * s)))
        out.append(self.rect(-0.2, 1.72, 0.08, 0.35, fill="#9ca3a6"))
        out.append(self.rect(-0.12, 0.52, 0.58, 0.35, fill=PKG, stroke=EDGE, stroke_width="1.25", rx=fmt(0.02 * s)))
        out.append(f'<circle cx="{fmt(X(0.02))}" cy="{fmt(Y(0.345))}" r="{fmt(0.09 * s)}" fill="#3a3f44"/>')
        # Reset switch.
        out.append(self.rect(0.1, 2.06, 0.25, 0.24, fill=METAL, stroke=EDGE, stroke_width="1"))
        out.append(f'<circle cx="{fmt(X(0.225))}" cy="{fmt(Y(1.94))}" r="{fmt(0.065 * s)}" fill="#f4f1e8" stroke="{EDGE}" stroke-width="0.75"/>')
        # 16U2 QFN, its crystal, its ICSP.
        out.append(self.rect(0.69, 1.40, 0.2, 0.2, fill=PKG, rx=fmt(0.01 * s)))
        out.append(self.rect(0.53, 1.10, 0.42, 0.18, fill=METAL, stroke=EDGE, stroke_width="1", rx=fmt(0.09 * s)))
        for c in range(3):
            for r in range(2):
                out.append(self.rect(0.565 + c * 0.1, 1.87 - r * 0.1, 0.09, 0.09, fill=PKG))
                out.append(self.rect(0.595 + c * 0.1, 1.84 - r * 0.1, 0.03, 0.03, fill=GOLD))
        # Regulator, caps, diode.
        out.append(self.rect(0.15, 0.78, 0.1, 0.16, fill=METAL))
        out.append(self.rect(0.24, 0.79, 0.17, 0.19, fill=PKG))
        for cx in (0.72, 1.0):
            out.append(self.rect(cx - 0.12, 0.47, 0.24, 0.24, fill="#d9d9d4", stroke=EDGE, stroke_width="0.75"))
            out.append(f'<circle cx="{fmt(X(cx))}" cy="{fmt(Y(0.35))}" r="{fmt(0.11 * s)}" fill="{METAL}" stroke="{EDGE}" stroke-width="0.75"/>')
            out.append(f'<path d="M{fmt(X(cx - 0.11))} {fmt(Y(0.35))} A{fmt(0.11 * s)} {fmt(0.11 * s)} 0 0 1 {fmt(X(cx - 0.055))} {fmt(Y(0.445))} L{fmt(X(cx - 0.055))} {fmt(Y(0.255))} A{fmt(0.11 * s)} {fmt(0.11 * s)} 0 0 1 {fmt(X(cx - 0.11))} {fmt(Y(0.35))}Z" fill="{PKG}"/>')
        out.append(self.rect(0.73, 0.155, 0.26, 0.085, fill=PKG))
        # ATmega328P DIP-28 in its socket, notch toward the right edge.
        out.append(self.rect(1.1, 0.87, 1.45, 0.44, fill="#15181a", rx=fmt(0.01 * s)))
        for i in range(14):
            lx = 1.185 + i * 0.1
            out.append(self.rect(lx - 0.013, 0.86, 0.026, 0.05, fill=METAL))
            out.append(self.rect(lx - 0.013, 0.48, 0.026, 0.05, fill=METAL))
        out.append(self.rect(1.115, 0.81, 1.42, 0.28, fill=PKG, stroke=EDGE, stroke_width="0.75"))
        out.append(f'<path d="M{fmt(X(2.535))} {fmt(Y(0.705))} A{fmt(0.035 * s)} {fmt(0.035 * s)} 0 0 0 {fmt(X(2.535))} {fmt(Y(0.635))}" fill="#101316"/>')
        out.append(f'<text x="{fmt(X(1.825))}" y="{fmt(Y(0.65))}" text-anchor="middle" font-family="{FONT}" font-size="{fmt(0.065 * s)}" fill="#aeb4b8">ATMEGA328P</text>')
        # 328P ICSP (2 x 3), op-amp, LEDs.
        for c in range(2):
            for r in range(3):
                out.append(self.rect(2.455 + c * 0.1, 1.2 - r * 0.1, 0.09, 0.09, fill=PKG))
                out.append(self.rect(2.485 + c * 0.1, 1.17 - r * 0.1, 0.03, 0.03, fill=GOLD))
        out.append(self.rect(2.3, 1.14, 0.08, 0.19, fill=PKG))
        for lx, ly, col in ((1.085, 1.61, "#f0b64b"), (1.085, 1.39, "#f0b64b"), (1.085, 1.30, "#f0b64b"), (2.305, 1.39, "#7bd36b")):
            out.append(self.rect(lx - 0.03, ly + 0.015, 0.06, 0.03, fill=col, stroke=EDGE, stroke_width="0.5"))
        # Headers: black strips with gold sockets.
        for row, pins in ((2.0, TOP[:10]), (2.0, TOP[10:]), (0.1, BOTTOM[:8]), (0.1, BOTTOM[8:])):
            a, b = pins[0][0], pins[-1][0]
            out.append(self.rect(a - 0.05, row + 0.05, b - a + 0.1, 0.1, fill="#17191b", rx=fmt(0.006 * s)))
            for p in pins:
                out.append(self.rect(p[0] - 0.02, row + 0.02, 0.04, 0.04, fill=GOLD))
        # Silk: model box and group legends.
        out.append(self.rect(1.23, 1.19, 0.54, 0.15, fill="none", stroke=SILK, stroke_width=fmt(0.012 * s)))
        out.append(f'<path d="M{fmt(X(1.5))} {fmt(Y(1.19))} V{fmt(Y(1.04))}" stroke="{SILK}" stroke-width="{fmt(0.012 * s)}"/>')
        for tx, txt in ((1.365, "UNO"), (1.635, "R3")):
            out.append(f'<text x="{fmt(X(tx))}" y="{fmt(Y(1.075))}" text-anchor="middle" font-family="{FONT}" font-size="{fmt(0.09 * s)}" font-weight="700" fill="{SILK}">{txt}</text>')
        if silk_labels:
            fs = fmt(0.045 * s)
            for p in TOP:
                if p[1]:
                    out.append(f'<text transform="translate({fmt(X(p[0]) + 0.016 * s)} {fmt(Y(1.9))}) rotate(-90)" text-anchor="end" font-family="{FONT}" font-size="{fs}" font-weight="700" fill="{SILK}">{p[1]}</text>')
            for p in BOTTOM:
                if p[1]:
                    out.append(f'<text transform="translate({fmt(X(p[0]) + 0.016 * s)} {fmt(Y(0.2))}) rotate(-90)" text-anchor="start" font-family="{FONT}" font-size="{fs}" font-weight="700" fill="{SILK}">{p[1]}</text>')
            for tx, ty, txt in ((1.45, 1.66, "DIGITAL (PWM~)"), (1.45, 0.395, "POWER"), (2.25, 0.395, "ANALOG IN"), (2.53, 1.29, "ICSP"), (1.02, 1.585, "L"), (1.02, 1.365, "TX"), (1.02, 1.275, "RX"), (2.38, 1.365, "ON")):
                out.append(f'<text x="{fmt(X(tx))}" y="{fmt(Y(ty))}" text-anchor="middle" font-family="{FONT}" font-size="{fs}" font-weight="700" fill="{SILK}">{txt}</text>')
        return "\n".join(out)


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


def board_svg():
    b = Board(120, 48, 24)
    w, h = fmt(2.7 * 120 + 72), fmt(2.1 * 120 + 48)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}">\n'
            f'<title>UNO R3 top illustration</title>\n'
            f'<desc>Flat vector of the ATmega328P board, top view, USB-B and DC jack on the left, digital header top, power and analog headers bottom. No vendor logos.</desc>\n'
            f'<rect width="{w}" height="{h}" fill="#FFFFFF"/>\n{b.draw()}\n</svg>\n')


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
           b.draw(silk_labels=False)]
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
# page embeds byte-identical copies from docs/assets. `--check` fails when any
# copy differs from what this script generates.
DOCS_ASSETS = HERE.parents[2] / "docs" / "assets" / "boards" / "arduino-uno"

if __name__ == "__main__":
    import sys

    outputs = {"board.svg": board_svg(), "pinout.svg": pinout_svg()}
    targets = [HERE / n for n in outputs] + [DOCS_ASSETS / n for n in outputs]
    if "--check" in sys.argv:
        stale = [str(t) for t in targets if not t.exists() or t.read_text(encoding="utf-8") != outputs[t.name]]
        if stale:
            sys.exit("stale: " + ", ".join(stale) + "\nrun: python3 examples/arduino-uno-blinky/images/generate.py")
        print("uno images up to date")
        sys.exit(0)
    DOCS_ASSETS.mkdir(parents=True, exist_ok=True)
    for target in targets:
        target.write_text(outputs[target.name], encoding="utf-8")
        print(f"wrote {target} ({target.stat().st_size} bytes)")
