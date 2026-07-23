#!/usr/bin/env python3
"""Patch framework-arduinoststm32 series detection for STM32WBA.

Upstream platformio-build.py uses mcu_type[:7] → "STM32WBxx" for stm32wba*,
which pulls the classic WB HAL and fails to compile. This one-shot patch
selects STM32WBAxx when the MCU id contains "stm32wba".
"""
from pathlib import Path
import os
import sys

def candidates():
    home = Path.home() / ".platformio/packages"
    for p in home.glob("framework-arduinoststm32*/tools/platformio/platformio-build.py"):
        yield p

def patch(path: Path) -> bool:
    text = path.read_text(encoding="utf-8")
    if 'if "stm32wba" in mcu:' in text and 'STM32WBAxx' in text:
        print(f"ok (already): {path}")
        return False
    needle = 'series = mcu_type[:7].upper() + "xx"'
    if needle not in text:
        print(f"skip (no needle): {path}", file=sys.stderr)
        return False
    repl = (
        '# LabWired: STM32WBA is a 3-letter family; [:7] would yield STM32WBxx.\n'
        'if "stm32wba" in mcu:\n'
        '    series = "STM32WBAxx"\n'
        'else:\n'
        '    series = mcu_type[:7].upper() + "xx"'
    )
    path.write_text(text.replace(needle, repl, 1), encoding="utf-8")
    print(f"patched: {path}")
    return True

def main() -> int:
    found = list(candidates())
    if not found:
        print("no framework-arduinoststm32 package found", file=sys.stderr)
        return 1
    for p in found:
        patch(p)
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
