[← Back to Hub](../README.md)

# VS Code UI Demo Checklist (Release Gate)

Use this checklist right before any external demo. It validates the UI path that is not fully covered by backend smoke tests.

## 0) Prepare Demo Artifacts

Run the scripted dry run first:

```bash
python3 ai/tests/demo_dry_run.py --mode fallback --device LM75B --docker
```

This produces a ready demo project in:

`ai/tests/demo_dry_run_output/project/`

## 1) Local Mode UI Pass

1. Open `ai/tests/demo_dry_run_output/project/` in VS Code.
2. Ensure `.vscode/launch.json` contains a LabWired config with:
   `type=labwired`, `request=launch`, `program=<firmware ELF>`, `systemConfig=system.yaml`.
3. Start debugging (F5) with `LabWired: Launch`.
4. Verify `stopOnEntry` behavior (session pauses on entry).
5. Set a breakpoint in firmware source, press Continue, verify breakpoint is hit.
6. Step over at least 3 instructions and verify PC/registers update.
7. Open `LabWired: Show Memory Inspector` and verify memory loads for a known address.
8. Open `LabWired: Show Command Center` and verify telemetry updates while running.

## 2) Docker Mode UI Pass

Set VS Code settings:

```json
{
  "labwired.executionMode": "docker",
  "labwired.docker.image": "w1ne/labwired-dev:latest",
  "labwired.docker.autoPull": true
}
```

Then:

1. Start `LabWired: Launch` again.
2. Verify debug output shows DAP starting in Docker.
3. Verify breakpoint hit + stepping still work.
4. Verify memory inspector still reads values.

## 3) Sign-Off

- [ ] Local mode pass complete
- [ ] Docker mode pass complete
- [ ] Breakpoint + step + registers verified
- [ ] Memory inspector verified
- [ ] Command Center telemetry verified

Record:

- Date:
- Operator:
- Commit:
- Notes / known issues:
