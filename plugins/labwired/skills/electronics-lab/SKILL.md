---
name: electronics-lab
description: Design an electronic circuit, select supported microcontroller parts, simulate firmware, or turn a circuit into a PCB layout with design-rule checks.
---

Use the hosted LabWired tools for the hardware task the user asked for. Check the board, component and pin identifiers in the catalog before wiring them. If a required component or physical behavior is not modeled, say which part of the task cannot be tested.

For firmware, agree on an observable result such as a serial message, output pin change or displayed value. Compile and run the firmware against the modeled circuit. Report a verified result only when the simulation assertion passes; include the observed value or run evidence when available.

For a PCB, distinguish schematic checks, routing completion and independent KiCad design-rule results. Report the checks actually performed. A passing model or board-rule check does not establish physical performance, fabrication quality or hardware safety.
