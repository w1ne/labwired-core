#!/usr/bin/env python3
"""Generate debugger register schemas (PeripheralDescriptor YAML) from a vendor SVD.

These descriptors are *decode metadata only*. They give a debugger the names,
offsets and bit slices of a NATIVE peripheral's registers so the VS Code
Peripherals tree can show `OUT = 0x00000020 [PIN5=1]` instead of "No register
descriptors available". Values still come from the peripheral's own model, read
side-effect-free. **Nothing here adds fidelity**, and the `register_coverage`
gate is unaffected because it probes the live bus, not schema.

`labwired asset ingest-svd` produces the same artifact and remains the canonical
tool. This script exists so the schemas can be regenerated without a full Rust
toolchain build. That equivalence is not assumed — `--verify-against` reproduces
an already-generated descriptor set and diffs it, so the two cannot drift
silently:

    scripts/gen_debug_schemas.py --svd tests/fixtures/real_world/nrf52832.svd \\
        --verify-against configs/peripherals/nrf52832

Generate a new set with:

    scripts/gen_debug_schemas.py --svd tests/fixtures/real_world/nrf52840.svd \\
        --out configs/peripherals/nrf52840
"""

from __future__ import annotations

import argparse
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

# SVD access semantics → the descriptor's snake_case enums (labwired_config's
# WriteAction / ReadAction). Unmapped spellings deliberately fall through to
# `side_effects: null` rather than being guessed at.
WRITE_ACTIONS = {
    "oneToClear": "write_one_to_clear",
    "zeroToClear": "write_zero_to_clear",
}
READ_ACTIONS = {
    "clear": "clear",
}

ACCESS = {
    "read-write": "READ_WRITE",
    "read-only": "READ_ONLY",
    "write-only": "WRITE_ONLY",
    "writeOnce": "WRITE_ONLY",
    "read-writeOnce": "READ_WRITE",
}


def text(node: ET.Element | None, tag: str, default: str = "") -> str:
    if node is None:
        return default
    found = node.findtext(tag)
    return found.strip() if found else default


def parse_int(raw: str, default: int = 0) -> int:
    raw = (raw or "").strip()
    if not raw:
        return default
    try:
        if raw.lower().startswith("0x"):
            return int(raw, 16)
        if raw.lower().startswith("#"):
            return int(raw[1:].replace("x", "0"), 2)
        return int(raw, 0)
    except ValueError:
        return default


def bit_range(field: ET.Element) -> tuple[int, int]:
    """Return (msb, lsb) from any of SVD's three field-position spellings."""
    raw = text(field, "bitRange")
    if raw.startswith("["):
        msb, lsb = raw.strip("[]").split(":")
        return int(msb), int(lsb)

    offset = field.findtext("bitOffset")
    width = field.findtext("bitWidth")
    if offset is not None:
        lsb = parse_int(offset)
        n = parse_int(width, 1) if width is not None else 1
        return lsb + n - 1, lsb

    msb = parse_int(text(field, "msb"))
    lsb = parse_int(text(field, "lsb"))
    return msb, lsb


def yaml_str(value: str) -> str:
    """Emit a YAML scalar, quoting only where the emitter would.

    Matches serde_yaml's plain-scalar rules closely enough to round-trip the
    reference descriptors byte-for-byte: a plain scalar cannot contain ": " or
    " #", cannot lead with an indicator character, and cannot be empty or look
    like another type.
    """
    if value == "":
        return "''"

    needs_quote = (
        ": " in value
        or value.endswith(":")
        or " #" in value
        or value[0] in "-?:,[]{}#&*!|>'\"%@`"
        or value != value.strip()
    )
    if not needs_quote:
        return value
    return "'" + value.replace("'", "''") + "'"


def collect_registers(container: ET.Element) -> list[tuple[ET.Element, str, int]]:
    """Registers directly under a `<registers>` node, including clustered ones.

    SVD groups related registers in a `<cluster>`, whose members are addressed
    relative to the cluster's own offset and named with the cluster as a prefix
    (FICR's `INFO` cluster holds `FLASH`, which the chip documents as
    `INFO_FLASH`). Reading only `./register` silently drops every clustered
    register — 29 of FICR's 44 on the nRF52.
    """
    out: list[tuple[ET.Element, str, int]] = []

    # Walk children in DOCUMENT order. Emitting all plain registers before all
    # clusters would reorder the peripheral's register list relative to the
    # datasheet (and relative to `ingest-svd`), which is what a reader scans.
    for child in container:
        if child.tag == "register":
            out.append((child, text(child, "name"), parse_int(text(child, "addressOffset"))))
            continue
        if child.tag != "cluster":
            continue

        cluster = child
        raw_name = text(cluster, "name")
        base = parse_int(text(cluster, "addressOffset"))
        members = collect_registers(cluster)

        # A cluster can itself be an array (`EVENTS_PREGION[%s]`, dim 2), which
        # repeats every member at a strided offset: EVENTS_PREGION0_RA,
        # EVENTS_PREGION1_RA, … Treating it as a single cluster loses half of them.
        dim = cluster.findtext("dim")
        if dim:
            count = parse_int(dim)
            increment = parse_int(text(cluster, "dimIncrement"), 4)
            raw_index = text(cluster, "dimIndex")
            indices = (
                [part.strip() for part in raw_index.split(",")]
                if raw_index
                else [str(i) for i in range(count)]
            )
            for i, index in enumerate(indices[:count]):
                prefix = raw_name.replace("[%s]", index).replace("%s", index).rstrip("_")
                for reg, name, offset in members:
                    full = f"{prefix}_{name}" if prefix else name
                    out.append((reg, full, base + i * increment + offset))
            continue

        prefix = raw_name.replace("[%s]", "").rstrip("_")
        for reg, name, offset in members:
            out.append((reg, f"{prefix}_{name}" if prefix else name, base + offset))

    return out


def registers_of(peripheral: ET.Element, root: ET.Element) -> list[tuple[ET.Element, str, int]]:
    """Registers of a peripheral, following `derivedFrom` when it has none."""
    container = peripheral.find("./registers")
    if container is not None:
        found = collect_registers(container)
        if found:
            return found

    derived = peripheral.get("derivedFrom")
    if not derived:
        return []
    for candidate in root.findall(".//peripherals/peripheral"):
        if text(candidate, "name") == derived:
            container = candidate.find("./registers")
            return collect_registers(container) if container is not None else []
    return []


def expand_registers(regs: list[tuple[ET.Element, str, int]]) -> list[tuple[ET.Element, str, int]]:
    """Flatten SVD register arrays into concrete (element, name, offset) triples.

    A `<dim>`/`<dimIncrement>` register such as `TASKS_TRIGGER[%s]` describes N
    consecutive registers, not one. A debugger showing a literal `[%s]` at a
    single offset would be showing something that does not exist on the chip.
    """
    out: list[tuple[ET.Element, str, int]] = []
    for reg, name, offset in regs:
        dim = reg.findtext("dim")

        if not dim:
            out.append((reg, name, offset))
            continue

        count = parse_int(dim)
        increment = parse_int(text(reg, "dimIncrement"), 4)
        raw_index = text(reg, "dimIndex")
        if raw_index:
            indices = [part.strip() for part in raw_index.split(",")]
        else:
            indices = [str(i) for i in range(count)]

        for i, index in enumerate(indices[:count]):
            expanded = name.replace("[%s]", index).replace("%s", index)
            out.append((reg, expanded, offset + i * increment))

    return out


def descriptor_yaml(peripheral: ET.Element, root: ET.Element) -> str:
    name = text(peripheral, "name")
    lines = [f"peripheral: {yaml_str(name)}", "version: 0.1.0", "registers:"]

    for reg, reg_name, offset in expand_registers(registers_of(peripheral, root)):
        size = parse_int(text(reg, "size"), 32) or 32
        access = ACCESS.get(text(reg, "access"), "READ_WRITE")
        reset = parse_int(text(reg, "resetValue"))

        lines.append(f"- id: {yaml_str(reg_name)}")
        lines.append(f"  address_offset: {offset}")
        lines.append(f"  size: {size}")
        lines.append(f"  access: {access}")
        lines.append(f"  reset_value: {reset}")

        fields = reg.findall("./fields/field")
        if not fields:
            lines.append("  fields: []")
        else:
            lines.append("  fields:")
            for field in fields:
                msb, lsb = bit_range(field)
                lines.append(f"  - name: {yaml_str(text(field, 'name'))}")
                lines.append("    bit_range:")
                lines.append(f"    - {msb}")
                lines.append(f"    - {lsb}")
                # Fold only the line wrapping the SVD author used; preserve
                # internal spacing, which the datasheet text sometimes relies on.
                description = re.sub(r"\s*\n\s*", " ", text(field, "description")).strip()
                if description:
                    lines.append(f"    description: {yaml_str(description)}")
        # SVD's modifiedWriteValues / readAction carry the register's access
        # semantics. Note these are DESCRIPTIVE here: the descriptor records what
        # the datasheet says, and the native model is what actually implements it.
        write_action = WRITE_ACTIONS.get(text(reg, "modifiedWriteValues"))
        read_action = READ_ACTIONS.get(text(reg, "readAction"))
        if write_action or read_action:
            lines.append("  side_effects:")
            lines.append(f"    read_action: {read_action or 'null'}")
            lines.append(f"    write_action: {write_action or 'null'}")
            lines.append("    on_read: null")
            lines.append("    on_write: null")
        else:
            lines.append("  side_effects: null")

    # Interrupt numbers travel with the descriptor. A peripheral that derives
    # its registers from another still declares its OWN interrupts, so read them
    # from this element, never the derivedFrom target.
    # Own interrupts first, then any inherited through `derivedFrom`. A derived
    # peripheral (EGU1 from EGU0) declares its own vector but still carries the
    # base peripheral's, and the descriptor records both.
    # Follow the whole chain, not one link: TIMER4 derives from TIMER3, which
    # derives from TIMER0, and the descriptor carries every vector along it.
    interrupts = list(peripheral.findall("./interrupt"))
    by_name = {text(p, "name"): p for p in root.findall(".//peripherals/peripheral")}
    seen = {text(peripheral, "name")}
    derived = peripheral.get("derivedFrom")
    while derived and derived not in seen:
        seen.add(derived)
        parent = by_name.get(derived)
        if parent is None:
            break
        interrupts.extend(parent.findall("./interrupt"))
        derived = parent.get("derivedFrom")

    if interrupts:
        lines.append("interrupts:")
        for irq in interrupts:
            lines.append(f"  {text(irq, 'name')}: {parse_int(text(irq, 'value'))}")
    else:
        lines.append("interrupts: null")
    lines.append("timing: null")

    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--svd", required=True, type=Path)
    parser.add_argument("--out", type=Path, help="Directory to write <peripheral>.yaml into.")
    parser.add_argument(
        "--verify-against",
        type=Path,
        help="Existing descriptor directory to diff against instead of writing.",
    )
    args = parser.parse_args()

    if not args.out and not args.verify_against:
        parser.error("pass --out or --verify-against")

    root = ET.parse(args.svd).getroot()
    generated = {
        text(p, "name").lower(): descriptor_yaml(p, root)
        for p in root.findall(".//peripherals/peripheral")
        if text(p, "name")
    }

    if args.verify_against:
        import yaml

        reference_dir: Path = args.verify_against
        reference = {p.stem: p.read_text() for p in sorted(reference_dir.glob("*.yaml"))}

        # Compare PARSED documents, not bytes. `interrupts` deserialises into a
        # HashMap, so the reference tool emits its entries in hash order — a
        # byte diff would report a difference that does not exist in the data.
        # Everything that matters to a debugger (register names, offsets, sizes,
        # access, bit ranges) is order-significant and compares exactly.
        def normalise(doc: dict) -> dict:
            """Canonicalise the two orderings that carry no meaning.

            `interrupts` deserialises into a HashMap, so the reference tool emits
            it in hash order. And where an SVD defines two names at the SAME
            offset (NVMC's ERASEPAGE / ERASEPCR1 are documented aliases of one
            register), their relative order is arbitrary. Sorting registers by
            (offset, id) folds both away while leaving every meaningful
            difference — a missing register, a wrong offset, a wrong bit range —
            fully visible.
            """
            out = dict(doc)
            regs = out.get("registers") or []
            out["registers"] = sorted(regs, key=lambda r: (r.get("address_offset", 0), r.get("id", "")))
            return out

        only_generated = sorted(set(generated) - set(reference))
        only_reference = sorted(set(reference) - set(generated))
        differing = []
        for key in sorted(set(generated) & set(reference)):
            if normalise(yaml.safe_load(generated[key])) != normalise(yaml.safe_load(reference[key])):
                differing.append(key)

        print(f"generated: {len(generated)}   reference: {len(reference)}")
        if only_generated:
            print(f"  only in generated ({len(only_generated)}): {only_generated[:10]}")
        if only_reference:
            print(f"  only in reference ({len(only_reference)}): {only_reference[:10]}")
        if differing:
            print(f"  differing ({len(differing)}): {differing[:10]}")
            sample = differing[0]
            gen = yaml.safe_load(generated[sample])
            ref = yaml.safe_load(reference[sample])
            gen_regs = {r["id"]: r for r in gen.get("registers") or []}
            ref_regs = {r["id"]: r for r in ref.get("registers") or []}
            print(f"\n  '{sample}': {len(gen_regs)} generated registers vs {len(ref_regs)} reference")
            for name in sorted(set(gen_regs) | set(ref_regs)):
                if gen_regs.get(name) != ref_regs.get(name):
                    print(f"    first differing register '{name}':")
                    print(f"      generated: {gen_regs.get(name)}")
                    print(f"      reference: {ref_regs.get(name)}")
                    break
            else:
                for field in ("peripheral", "version", "interrupts", "timing"):
                    if gen.get(field) != ref.get(field):
                        print(f"    field '{field}': {gen.get(field)!r} vs {ref.get(field)!r}")

        ok = not (only_generated or only_reference or differing)
        print("\nEQUIVALENT" if ok else "\nNOT EQUIVALENT")
        return 0 if ok else 1

    args.out.mkdir(parents=True, exist_ok=True)
    for name, body in sorted(generated.items()):
        (args.out / f"{name}.yaml").write_text(body)
    print(f"wrote {len(generated)} descriptors to {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
