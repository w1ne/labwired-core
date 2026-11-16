"""Write sd.img: a 4 MiB FAT16 volume (no partition table, as many SD cards
used with 3D printers are formatted) holding one G-code file, TEST.GCO.

Usage: python3 make_sd_image.py [out.img]
"""
import struct
import sys

SECTOR = 512
TOTAL = 8192                 # 4 MiB
RESERVED = 1
FATS = 2
ROOT_ENTRIES = 512
SPC = 1                      # sectors per cluster -> ~8100 clusters, FAT16
FAT_SECTORS = 32             # 32 * 256 entries >= cluster count + 2
GCODE = b"; LabWired SD test file\nG28\nG1 X10 Y10 F3000\nM114\n"


def build():
    img = bytearray(TOTAL * SECTOR)
    bs = bytearray(SECTOR)
    bs[0:3] = b"\xEB\x3C\x90"
    bs[3:11] = b"LABWIRED"
    struct.pack_into("<HBHBHHBHHHII", bs, 11, SECTOR, SPC, RESERVED, FATS,
                     ROOT_ENTRIES, TOTAL, 0xF8, FAT_SECTORS, 63, 255, 0, 0)
    struct.pack_into("<BBBI11s8s", bs, 36, 0x80, 0, 0x29, 0x1234ABCD,
                     b"LABWIRED SD", b"FAT16   ")
    bs[510:512] = b"\x55\xAA"
    img[0:SECTOR] = bs
    fat0 = RESERVED * SECTOR
    root = (RESERVED + FATS * FAT_SECTORS) * SECTOR
    data = root + ROOT_ENTRIES * 32
    clusters = (len(GCODE) + SECTOR * SPC - 1) // (SECTOR * SPC)
    for f in range(FATS):
        fat = fat0 + f * FAT_SECTORS * SECTOR
        struct.pack_into("<HH", img, fat, 0xFFF8, 0xFFFF)
        for c in range(clusters):
            nxt = 0xFFFF if c == clusters - 1 else 3 + c
            struct.pack_into("<H", img, fat + (2 + c) * 2, nxt)
    # Volume label entry, then the file.
    img[root:root + 11] = b"LABWIRED SD"
    img[root + 11] = 0x08
    ent = root + 32
    img[ent:ent + 11] = b"TEST    GCO"
    img[ent + 11] = 0x20
    struct.pack_into("<HI", img, ent + 26, 2, len(GCODE))
    img[data:data + len(GCODE)] = GCODE
    return img


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "sd.img"
    with open(out, "wb") as fh:
        fh.write(build())
    print(f"wrote {out}")
