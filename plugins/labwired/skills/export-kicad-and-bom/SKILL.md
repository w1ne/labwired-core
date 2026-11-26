---
name: export-kicad-and-bom
description: Export fabrication-ready KiCad schematics, autorouted KiCad PCBs, Gerber/drill packs, BOM/CPL spreadsheets with live supplier pricing, and 3D STEP enclosures via LabWired MCP.
---

# Export KiCad and BOM

Use this skill when a circuit is validated and ready to transition from simulation to real-world PCB fabrication and assembly.

## Workflow

### 1. Pre-Export Electrical Gate
Never generate Gerbers from an unvalidated netlist:
- Call `labwired_validate` on the active diagram and resolve every error.
- If firmware is part of the design, confirm `labwired_verify` passes so pin assignments are locked before copper routing.

### 2. Create, Place, and Autoroute the KiCad PCB
- **New board from LabWired diagram:**
  - Fast path: call `labwired_pcb_auto` to generate the board outline, place footprints, autoroute copper, and run KiCad DRC in one step.
  - Step-by-step control: `labwired_pcb_create` -> `labwired_pcb_place` -> `labwired_pcb_route` -> `labwired_pcb_attach_schematic`.
- **Existing KiCad design:**
  - Import with `labwired_import`, validate with `labwired_pcb_validate_imported`, and autoroute with `labwired_pcb_route_imported`.
- Call `labwired_pcb_inspect` and `labwired_pcb_view` to confirm:
  - 100% routed nets (0 unrouted ratsnest airwires)
  - 0 KiCad DRC errors (clearance, track width, drill/annular ring, courtyard overlaps)

### 3. Export Schematic, Netlist, PCB, and Gerber Fab Pack
- Call `labwired_export` for KiCad schematic (`.kicad_sch`), netlist (`.net`), BOM CSV, and SVG documentation.
- Call `labwired_pcb_export` for the routed `.kicad_pcb`, Gerber copper/mask/silk/outline layers, Excellon drill files, and pick-and-place CPL files ready for JLCPCB, PCBWay, or Eurocircuits.

### 4. Generate Priced BOM & Check Stock
- Call `labwired_price` to produce a priced Bill of Materials with real MPNs, distributor links, unit/extended pricing, and stock levels.
- If any part has zero stock or excessive lead time, search replacements via `labwired_part_search` and verify pin/footprint compatibility before re-exporting.
- Optionally call `labwired_order` to produce a turnkey fabrication + assembly cost estimate (never submit payment without explicit user confirmation).

### 5. Optional: Mechanical 3D STEP Enclosure (KernelCAD)
- Call `labwired_cad_tools` and `labwired_cad_call` to generate a parametric enclosure matching board dimensions, mounting holes, USB/connector cutouts, and display/LED windows, exported as STEP/STL.

## Handoff Summary
Deliver a clean fabrication manifest listing:
- Schematic & PCB files (`.kicad_sch`, `.kicad_pcb`) + DRC status
- Gerber/drill/CPL archive status
- Priced BOM table (quantity, refdes, MPN, unit price, stock status, total)
- Mechanical 3D STEP/enclosure file (if generated)
