#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

target="thumbv6m-none-eabi"
crate="firmware-pico-w-bme280-smoke"
elf="target/${target}/release/${crate}"
cargo build -p "$crate" --release --target "$target"

# RP2040 flash has a 256-byte stage-2 boot2 prefix. The simulator smoke fast-
# boots the same register-level routine used after reset, so bind ELF entry to
# that named function while leaving the board's reset_vector_offset truthful.
python3 - "$elf" <<'PY'
import struct
import sys
from pathlib import Path

path = Path(sys.argv[1])
data = bytearray(path.read_bytes())
if data[:4] != b"\x7fELF" or data[4] != 1 or data[5] != 1:
    raise SystemExit(f"expected a 32-bit little-endian ELF: {path}")

section_offset = struct.unpack_from("<I", data, 32)[0]
section_size, section_count = struct.unpack_from("<HH", data, 46)

def section(index):
    start = section_offset + index * section_size
    return struct.unpack_from("<IIIIIIIIII", data, start)

entry_address = None
for index in range(section_count):
    _, kind, _, _, offset, size, link, _, _, entry_size = section(index)
    if kind != 2 or entry_size == 0:  # SHT_SYMTAB
        continue
    strings = section(link)
    string_data = data[strings[4]:strings[4] + strings[5]]
    for symbol_offset in range(offset, offset + size, entry_size):
        name_offset, value, _, _, _, section_index = struct.unpack_from(
            "<IIIBBH", data, symbol_offset
        )
        if section_index == 0:
            continue
        end = string_data.find(b"\0", name_offset)
        if string_data[name_offset:end] == b"labwired_entry":
            entry_address = value | 1  # Cortex-M Thumb entry bit.
            break
    if entry_address is not None:
        break

if entry_address is None:
    raise SystemExit(f"labwired_entry symbol missing from {path}")
struct.pack_into("<I", data, 24, entry_address)
path.write_bytes(data)
print(f"set fast-boot ELF entry to labwired_entry at 0x{entry_address:08x}")
PY
