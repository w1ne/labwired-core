# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""Regression tests for control-character escaping in gen_debug_schemas.py.

ST's SVDs carry raw C1 control characters (U+0080..U+009F) inside register
descriptions. serde_yaml 0.9.34 — the parser the debugger loads
`PeripheralDescriptor` YAML with — rejects a document containing one
("control characters are not allowed"), so the first STM32U575 generation
produced 12 descriptors the runtime could not read. `yaml_str` now falls back
to a double-quoted scalar carrying the same escape sequences the Rust ingestor
(`svd-ingestor`) emits, so the value survives the round trip.
"""

import sys
import xml.etree.ElementTree as ET
from pathlib import Path

import pytest
import yaml

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gen_debug_schemas as gds  # noqa: E402

# The C1 controls the STM32U575 SVD actually carries, plus DEL, the C0 escapes
# with special spellings, and the line-break code points libyaml refuses to
# write raw.
CONTROL_CHARS = [
    "\x00",
    "\x1f",
    "\t",
    "\n",
    "\r",
    "\x7f",
    "\x80",
    "\x82",
    "\x85",
    "\x89",
    "\x99",
    "\x9c",
    "\x9d",
    "\u2028",
    "\u2029",
]


@pytest.mark.parametrize("char", CONTROL_CHARS)
def test_yaml_str_round_trips_control_characters(char):
    value = f"pre{char}post"
    scalar = gds.yaml_str(value)
    assert yaml.safe_load(f"v: {scalar}")["v"] == value


def test_yaml_str_escape_spelling_matches_libyaml():
    assert gds.yaml_str("DSIZE \x89 8 bit") == '"DSIZE \\x89 8 bit"'
    assert gds.yaml_str("a\x80b") == '"a\\x80b"'
    assert gds.yaml_str("a\tb") == '"a\\tb"'
    assert gds.yaml_str("a\nb") == '"a\\nb"'
    assert gds.yaml_str("a\rb") == '"a\\rb"'
    assert gds.yaml_str('a\x89"b\\c') == '"a\\x89\\"b\\\\c"'


def test_yaml_str_leaves_ordinary_text_unchanged():
    assert gds.yaml_str("") == "''"
    assert gds.yaml_str("plain text") == "plain text"
    assert gds.yaml_str("MB: field") == "'MB: field'"
    assert gds.yaml_str("'quoted'") == "'''quoted'''"
    # Non-ASCII text that is printable stays raw, as serde_yaml emits it.
    assert gds.yaml_str("DSIZE \xa4 8 bit") == "DSIZE \xa4 8 bit"


def _descriptor_document(description: str) -> dict:
    """A minimal SPI2 descriptor run through the real generator path."""
    device = ET.Element("device")
    peripheral = ET.SubElement(ET.SubElement(device, "peripherals"), "peripheral")
    ET.SubElement(peripheral, "name").text = "SPI2"
    register = ET.SubElement(ET.SubElement(peripheral, "registers"), "register")
    ET.SubElement(register, "name").text = "CFG1"
    ET.SubElement(register, "addressOffset").text = "0x0"
    ET.SubElement(register, "size").text = "32"
    field = ET.SubElement(ET.SubElement(register, "fields"), "field")
    ET.SubElement(field, "name").text = "DSIZE"
    ET.SubElement(field, "bitOffset").text = "0"
    ET.SubElement(field, "bitWidth").text = "5"
    ET.SubElement(field, "description").text = description

    return yaml.safe_load(gds.descriptor_yaml(peripheral, device))


def test_generated_descriptor_with_c1_controls_parses():
    description = "number of bits per frame; DSIZE \x89\xa4 8 bit"
    doc = _descriptor_document(description)
    assert doc["peripheral"] == "SPI2"
    assert doc["registers"][0]["fields"][0]["description"] == description


def test_parse_int_zero_padded_decimal_is_decimal():
    # ST's STM32U575 SVD zero-pads every single-digit interrupt value. `int(raw, 0)`
    # rejects '061' as a malformed octal literal and the old fallback returned 0.
    assert gds.parse_int("061") == 61
    assert gds.parse_int("002") == 2
    assert gds.parse_int("000") == 0
    # The other spellings the SVD uses must keep working.
    assert gds.parse_int("0x3D") == 61
    assert gds.parse_int("#111101") == 61
    assert gds.parse_int("0b111101") == 61
    assert gds.parse_int("61") == 61
    assert gds.parse_int(" 061 ") == 61
    assert gds.parse_int("", 7) == 7
    assert gds.parse_int("nonsense", 7) == 7


def test_generated_descriptor_parses_zero_padded_interrupt():
    device = ET.Element("device")
    peripheral = ET.SubElement(ET.SubElement(device, "peripherals"), "peripheral")
    ET.SubElement(peripheral, "name").text = "USART1"
    interrupt = ET.SubElement(peripheral, "interrupt")
    ET.SubElement(interrupt, "name").text = "USART1"
    ET.SubElement(interrupt, "value").text = "061"

    doc = yaml.safe_load(gds.descriptor_yaml(peripheral, device))
    assert doc["interrupts"] == {"USART1": 61}
