# LabWired agent hardware loop

Use LabWired as a deterministic virtual hardware lab for firmware work.

1. Choose a board with `labwired_list_boards`.
2. Validate custom system YAML with `labwired_validate_system`.
3. Build or update the diagram and validate it with `labwired_validate_diagram`.
4. Compile the diagram to a board manifest with `labwired_compile_diagram`. On success, the tool persists the manifest to `.labwired/boards/<name>.yaml` and returns a `board_path`. Use that path as the `--system` argument for `labwired_simulate`, or pass the returned `system_yaml` directly. Warnings are included in the response; errors abort compilation.
5. Compile firmware locally and pass the ELF as base64.
6. Run with `labwired_run_lab` for a preconfigured board or `labwired_simulate` for raw YAML control.
7. Inspect snapshots with `labwired_inspect_run`.
8. Use `labwired_fuzz` when firmware exposes the fuzz contract.
9. Iterate until serial, GPIO, cycle counts, and stop reasons match the intended behavior.

The local MCP server shells out to the `labwired` CLI. Set `LABWIRED_CLI` or `LABWIRED_REPO_ROOT` when running outside a checkout.

## Define missing components

If a part is not in the board catalog (`labwired_list_boards` /
`labwired_catalog`; the hosted connector also offers `labwired_list_components`),
define it yourself with
`labwired_define_component`: submit a declarative IR spec (register file,
pointer rule, observables) derived from the part's datasheet. The tool
validates the spec (stable `ICOMP_*` diagnostic codes with hints) and returns
a `spec_path` plus the exact `external_devices` manifest entry to use
(`type: ir`, `config.spec_path`). Reference specs:
`core/configs/components/pca9685.yaml` (pointer + auto-increment + observables)
and `core/configs/components/tmp102.yaml` (16-bit phased reads + update rules).
