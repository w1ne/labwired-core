# LabWired Simulation Safety Guidelines

This document provides guidance on configuring simulation limits to prevent memory exhaustion and system crashes.

## Overview

LabWired provides multiple safety mechanisms to prevent runaway simulations from consuming excessive resources:

| Limit | Purpose | Recommended Value | Maximum Safe Value |
|-------|---------|-------------------|-------------------|
| `max_steps` | Prevents infinite loops | 1,000,000 | 100,000,000 |
| `max_cycles` | Limits CPU cycle count | 10,000,000 | 1,000,000,000 |
| `max_uart_bytes` | Prevents excessive UART output | 1 MB (1,048,576) | 100 MB |
| `wall_time_ms` | Limits real-time execution | 10,000 (10s) | 300,000 (5min) |
| `no_progress_steps` | Detects stuck execution | 1,000 | 100,000 |
| `max_vcd_bytes` | Limits VCD trace file size | 100 MB | 1 GB |

## Safety Mechanisms

### 1. Execution Limits

**max_steps** (required)
- Limits the total number of simulation steps
- **Always set this** - it's your primary safety net
- For quick tests: 10,000 - 100,000 steps
- For standard tests: 1,000,000 steps
- For stress tests: 10,000,000 - 100,000,000 steps

**max_cycles** (optional)
- Limits total CPU cycles (includes peripheral wait states)
- Use when you need precise cycle-count budgeting
- Typically 10x higher than max_steps for simple code

**wall_time_ms** (optional)
- Hard real-time limit regardless of simulation progress
- Useful for CI/CD pipelines with strict time budgets
- Prevents hanging on pathological cases

### 2. Output Limits

**max_uart_bytes** (optional)
- Prevents memory exhaustion from excessive UART output
- Set to 1-10 MB for most tests
- Firmware with verbose logging may need higher limits

**max_vcd_bytes** (optional, interactive mode only)
- Limits VCD trace file size
- VCD files grow ~100-500 bytes per instruction
- For 1M instructions: expect 100-500 MB VCD file
- Recommended: 100 MB for debugging, 1 GB for detailed analysis

### 3. Hang Detection

**no_progress_steps** (optional)
- Detects when PC doesn't change (infinite loop at same address)
- Recommended: 1,000 steps for quick detection
- Set higher (10,000+) if firmware intentionally spins

## Configuration Examples

### Quick Validation Run (< 1 second)

```yaml
schema_version: "1.0"
inputs:
  firmware: "firmware.elf"
limits:
  max_steps: 10000
  wall_time_ms: 1000
  no_progress_steps: 100
```

### Standard Test Run (< 10 seconds)

```yaml
schema_version: "1.0"
inputs:
  firmware: "firmware.elf"
limits:
  max_steps: 1000000
  max_cycles: 10000000
  max_uart_bytes: 1048576  # 1 MB
  wall_time_ms: 10000
  no_progress_steps: 1000
```

### Long-Running Stress Test

```yaml
schema_version: "1.0"
inputs:
  firmware: "firmware.elf"
limits:
  max_steps: 100000000
  max_cycles: 1000000000
  max_uart_bytes: 104857600  # 100 MB
  wall_time_ms: 300000  # 5 minutes
  no_progress_steps: 10000
```

### Interactive Debugging with VCD

```bash
labwired --firmware firmware.elf \\
  --vcd trace.vcd \\
  --max-steps 1000000
```

> [!WARNING]
> VCD files can grow very large. A 1M step simulation can produce a 100-500 MB VCD file. Use `max_vcd_bytes` or limit `max_steps` when generating VCD traces.

## Troubleshooting

### Simulation Runs Out of Memory

**Symptoms**: System becomes unresponsive, OOM killer terminates process

**Solutions**:
1. Reduce `max_steps` to limit simulation length
2. Add `max_uart_bytes` to prevent UART buffer growth
3. Disable VCD output or add `max_vcd_bytes` limit
4. Add `wall_time_ms` as a hard timeout

### Simulation Hangs Indefinitely

**Symptoms**: Simulation runs but never completes

**Solutions**:
1. Add `wall_time_ms` for hard timeout
2. Add `no_progress_steps` to detect infinite loops
3. Reduce `max_steps` if firmware is expected to complete quickly

### VCD File Too Large

**Symptoms**: Disk fills up, VCD viewer crashes

**Solutions**:
1. Reduce `max_steps` to limit trace length
2. Add `max_vcd_bytes` limit (e.g., 100 MB)
3. Only enable VCD for specific debugging sessions
4. Use breakpoints to stop before generating excessive trace data

## Best Practices

1. **Always set max_steps**: This is your primary safety mechanism
2. **Use wall_time_ms in CI**: Prevents hung builds in automated pipelines
3. **Start conservative**: Begin with low limits and increase as needed
4. **Monitor resource usage**: Check memory and disk usage during long runs
5. **Combine multiple limits**: Use both step-based and time-based limits for defense in depth
6. **Test limits locally**: Verify limits work before deploying to CI/CD

## Pathological Cases

### Infinite Loop

```c
while(1) {
    // Stuck here forever
}
```

**Protection**: `no_progress_steps` detects PC not changing

### Verbose Logging

```c
while(1) {
    printf("Debug: iteration %d\\n", i++);
}
```

**Protection**: `max_uart_bytes` prevents memory exhaustion

### Long-Running Computation

```c
for(int i = 0; i < 1000000000; i++) {
    compute();
}
```

**Protection**: `max_steps` or `max_cycles` limits execution

### VCD Trace Explosion

```bash
labwired --firmware long_running.elf --vcd trace.vcd --max-steps 100000000
```

**Protection**: `max_vcd_bytes` limits file size (or reduce `max_steps`)

## Agent Usage

When running simulations programmatically, always set appropriate limits:

```python
import subprocess
import json

def run_safe_simulation(firmware_path, max_steps=1_000_000):
    """Run simulation with safe default limits"""
    script = {
        "schema_version": "1.0",
        "inputs": {
            "firmware": firmware_path
        },
        "limits": {
            "max_steps": max_steps,
            "max_cycles": max_steps * 10,
            "max_uart_bytes": 1_048_576,  # 1 MB
            "wall_time_ms": 30_000,  # 30 seconds
            "no_progress_steps": 1_000
        }
    }

    # Write script and run
    with open("test.yaml", "w") as f:
        yaml.dump(script, f)

    result = subprocess.run(
        ["labwired", "test", "--script", "test.yaml"],
        capture_output=True,
        timeout=60  # Additional OS-level timeout
    )

    return json.loads(result.stdout)
```

## Summary

- **Always use max_steps** - your primary safety net
- **Add wall_time_ms for CI/CD** - prevents hung builds
- **Limit UART output** - prevents memory exhaustion from logging
- **Be careful with VCD** - files can grow very large
- **Combine multiple limits** - defense in depth
- **Test limits locally first** - before deploying to production
