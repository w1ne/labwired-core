# LabWired agent hardware loop

Use LabWired as a deterministic virtual hardware lab for firmware work.

1. Choose a board with `labwired_list_boards`.
2. Discover modeled peripherals with `labwired_list_components`.
3. Build or update the diagram with an MCU, components, and wires.
4. Validate the diagram with `labwired_validate_diagram` before running.
5. Compile firmware outside hosted MCP using the documented scaffold and target flags.
6. Run the ELF with `labwired_run`.
7. Inspect serial output, cycle counts, stop reasons, and hardware diagnosis.
8. Iterate on firmware or wiring until simulator evidence matches the intended behavior.

Hosted MCP accepts Clerk OAuth bearer tokens and `lwk_live_` workspace API keys. The hosted connector runs firmware through the LabWired builder; it does not compile source.

## Define missing components

If a part is not in `labwired_list_components`, define it yourself with
`labwired_define_component`: submit a declarative IR spec (register file,
pointer rule, observables) derived from the part's datasheet. The tool
validates the spec (stable `ICOMP_*` diagnostic codes with hints) and returns
a `spec_path` plus the exact `external_devices` manifest entry to use
(`type: ir`, `config.spec_path`). Reference specs:
`core/configs/components/pca9685.yaml` (pointer + auto-increment + observables)
and `core/configs/components/tmp102.yaml` (16-bit phased reads + update rules).

Note: `labwired_define_component` is local-MCP-only for now; it is not available
through the hosted connector.
