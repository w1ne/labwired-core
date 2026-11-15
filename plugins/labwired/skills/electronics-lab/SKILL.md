---
name: electronics-lab
description: Design an electronic circuit, select supported microcontroller parts, simulate firmware, or turn a circuit into a PCB layout with design-rule checks.
---

Use the hosted LabWired tools for a hardware task the user asked for. A theory
question that needs only an explanation does not require a project or tool call.
Discover the tools available to this connection before choosing a workflow;
do not assume that the default endpoint exposes the expert catalog tools.

## Default project workflow

When available, use `labwired_hardware({ request })` to create one persistent
hardware project. Include the user's actual device, requested outputs and hard
constraints. The project tool handles catalog selection, wiring, firmware and
supported simulation or PCB stages internally. Do not stop at catalog discovery
or tell the user to assemble the design manually just because specialist tools
are not exposed on this connection.

For a revision, inspection or additional check, pass the exact returned
`project_id` to `labwired_hardware({ request, project_id })`. Reuse a returned
`session_id` when continuing that same conversation. Never invent identifiers,
replace an existing project after a failed lookup, or create a new project just
to inspect the same board in 3D.

Use specialist tools only when available and needed for a specific diagnostic
or expert request. In that path, check board, component and pin identifiers in
the catalog before wiring them. Do not silently substitute a different device.
If a required component or physical behavior is not modeled, identify the gap
and distinguish the completed supported work from the untested requirement.

## First usable deliverable

Return the actual project identifier and supplied open/share URL when available,
plus the status of the requested checks. Report a saved project, usable export
or verified run only when the tool result establishes it. A returned card or URL
does not prove that an embedded viewer rendered in Claude or Grok; on clients
without that viewer, provide the returned web link instead. If a stage is still
running or blocked, preserve the project identity and report that state.

For firmware, define an observable result such as a serial message, output pin
change or displayed value. Report a verified result only when the relevant
simulation assertion passes, and include the returned observed value or run
evidence. Compilation success is not verification. Failed and unavailable checks
remain separate from passing checks.

For a PCB, distinguish schematic checks, routing completion and independent
KiCad design-rule results. Associate files and checks with the exact returned
project/revision when the tool supplies one. Deliver manufacturing files only
when actually returned; do not infer them from a 3D preview. A passing model or
board-rule check does not establish physical performance, fabrication quality
or hardware safety.

Do not place orders, initiate checkout or charge anything as part of a design
or verification request. Follow the client's sign-in flow when authentication
is required; never request credentials in chat or write them into plugin files.
