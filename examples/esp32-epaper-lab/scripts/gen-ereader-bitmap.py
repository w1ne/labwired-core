#!/usr/bin/env python3
"""
Render the same content the Arduino labwired-ereader.ino paints
("LabWired Reader", body text, "Page 1") into two bit-packed 128x296
planes (black + red), MSB-first, and emit Rust const arrays the
esp32-epaper-lab can stream verbatim.

The panel is portrait native 128w x 296h but the Arduino sketch uses
display.setRotation(1) → landscape 296w x 128h. We mirror that here:
render at 296x128 landscape, then rotate 90° before bit-packing so the
SSD1680 sees the same byte stream as the panel-native portrait
orientation.
"""

from PIL import Image, ImageDraw, ImageFont

W, H = 296, 128  # landscape after rotation=1

# Three planes: WHITE (background), BLACK (text), RED (accent).
# We render at landscape, then rotate to portrait for the panel.
img = Image.new('RGB', (W, H), (255, 255, 255))
draw = ImageDraw.Draw(img)

# Use a default bitmap font that ships with PIL — small, monospace.
# 10pt looks similar to GxEPD2's FreeSerif9pt7b.
try:
    title_font = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf', 18)
    body_font = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf', 11)
    red_font = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf', 13)
except (OSError, IOError):
    title_font = ImageFont.load_default()
    body_font = ImageFont.load_default()
    red_font = ImageFont.load_default()

# Title
draw.text((10, 6), "LabWired Reader", fill=(0, 0, 0), font=title_font)

# Body
y = 35
for line in [
    "The simulator IS the",
    "hardware. Same firmware",
    "ELF runs in your browser,",
    "on your bench, and in CI.",
]:
    draw.text((10, y), line, fill=(0, 0, 0), font=body_font)
    y += 16

# Red accent: "Page 1" bottom-right
draw.text((W - 70, H - 22), "Page 1", fill=(255, 0, 0), font=red_font)

# Rotate to panel-native portrait (128w x 296h).
img = img.rotate(-90, expand=True)
assert img.size == (128, 296)

# Bit-pack to two 1bpp planes: black and red. MSB first.
# SSD1680 convention: bit = 0 means "paint this color"; bit = 1 means "leave white".
# So:
#   black_plane[byte] bit = 0 → ink black
#   red_plane[byte]   bit = 0 → ink red
# Each row is 128/8 = 16 bytes.
ROW_BYTES = 16
ROWS = 296

black = bytearray(ROW_BYTES * ROWS)
red = bytearray(ROW_BYTES * ROWS)
for y in range(ROWS):
    for xb in range(ROW_BYTES):
        bb = 0xFF
        rb = 0xFF
        for bit in range(8):
            x = xb * 8 + bit
            r, g, b = img.getpixel((x, y))
            if r < 128 and g < 128 and b < 128:
                bb &= ~(1 << (7 - bit)) & 0xFF
            elif r > 128 and g < 128 and b < 128:
                rb &= ~(1 << (7 - bit)) & 0xFF
        black[y * ROW_BYTES + xb] = bb
        red[y * ROW_BYTES + xb] = rb

def emit(name, data):
    print(f"pub const {name}: [u8; {len(data)}] = [")
    for i in range(0, len(data), 16):
        chunk = data[i:i + 16]
        print("    " + ", ".join(f"0x{b:02x}" for b in chunk) + ",")
    print("];")

emit("EREADER_BLACK_PLANE", bytes(black))
print()
emit("EREADER_RED_PLANE", bytes(red))
