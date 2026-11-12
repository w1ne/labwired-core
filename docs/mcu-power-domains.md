# Sampled MCU power domains

`ChipDescriptor.power_domains` describes distinct physical supply observations and a sampled reset supervisor. It is opt-in and mutually exclusive with the existing single-rail `supply_monitor`. Existing AT32 and ratio-only GPIO descriptors retain their old serialized form and single-rail behavior.

This API separates three facts:

1. A rail is inside a declared operating range.
2. A reset detector is asserted, delayed, or latched released.
3. A digital input threshold belongs to a declared nominal voltage mode.

A POR threshold below a guaranteed operating minimum does not authorize CPU execution below that minimum. A peripheral rail is not automatically a universal CPU execution rail. A nominal digital threshold is not a guarantee over the whole physical supply range.

## Descriptor contract

```yaml
power_domains:
  rails:
    core:
      nominal_v: 1.1
      min_v: 1.05
      max_v: 1.16
      pins: [CORE]
    io:
      nominal_v: 3.3
      min_v: 1.62
      max_v: 3.63
      pins: [IO]
    detector:
      nominal_v: 3.3
      min_v: 3.135
      max_v: 3.63
      pins: [DETECTOR]
  io_domain: io
  execution_domains: [core]
  system_reset_scope: core_only
  latched_por:
    sense_domain: core
    detector_domain: detector
    rearm_below_v: 1.1
    release_at_v: 0.957
    release_delay_ns: 3000
  brown_out:
    sense_domain: core
    detector_domain: detector
    enabled: true
    falling_v: 0.946
    rising_v: 0.95546
    startup_delay_ns: 55000
    assert_delay_ns: 3000
    release_delay_ns: 55000
    enable_delay_ns: 35000
    reprogram_delay_ns: 20000
    control_peripheral: power_controller
  peripherals:
    gpio_controller: {io: io}
    power_controller: {detector: detector}
```

This is the synthetic regression contract. Exact device descriptors must supply their own source-backed ranges, reset values, timing, pins, and supported peripheral roles. The generic engine contains no chip IDs, voltage-select encodings, or drone-specific assumptions.

The schema accepts 1–16 rails. Names contain 1–64 ASCII letters, digits, underscores, or hyphens. Every rail has finite `0 < min_v <= nominal_v <= max_v`. Optional physical pin labels are unique without case. Every execution domain, detector domain, peripheral role target, and I/O domain must resolve. Unknown keys, duplicate execution domains, invalid thresholds, and missing reset scope fail parsing.

`system_reset_scope` is explicit. `core_only` preserves chip detector configuration and chip reset flags on AIRCR SYSRESETREQ. `chip` requests a complete modeled chip reset. Core-local system-reset callbacks still run for `core_only`, so architectural state such as SysTick can reset while exact peripheral models preserve the wider chip state. Each exact profile must implement the correct callback scope.

## Routes and atomic publication

An analog model writes:

```text
board.power.<domain>_volts
node.<node-id>.board.power.<domain>_volts
```

The second form is the existing shared-World namespace. Binding accepts only declared domain names. Legacy `vdd` or `vdda` aliases do not create rails on a named-domain descriptor. Each rail has one producer. Conflicting rail sample intervals, unknown domains, and duplicate producers fail binding before execution. Unrelated faster models do not advance a slower rail supervisor. The router merges fresh outputs from all same-rate rail writers into one complete batch at the owning timestamp. Missing peer outputs or skipped owner intervals fail closed. The session schedules the actual earliest next model boundary, including periods that do not divide one another.

`SystemBus::apply_mcu_power_voltages(&[(String, f64)], time_ns)` validates the whole MCU batch before publishing observations. Values must be finite and nonnegative. Each declared route can occur once per batch. The simulation timeline must be monotonic. All rail values publish together, then peripheral `sync_power_domains` callbacks observe the complete batch. Invalid updates do not publish an earlier subset.

Attaching any sampled route changes the entire registry to measured mode. In that mode, every rail without an observation remains unknown and unavailable, including a peripheral handle whose own rail has no route. Missing values retain a previous observation; they never silently revert to nominal voltage. Before any sampled attachment, the registry explicitly assumes its descriptor's nominal supplies. This standalone assumption is exposed by `measured_mode: false`.

World construction primes the analog model and publishes its initial observations before CPU transactions. A zero-source initial state is distinct from a constant operating-point solve. This API does not turn a constant operating point into cold-start evidence.

## Peripheral capabilities

A descriptor role binds a read-only `PowerDomainHandle` through:

```rust
Peripheral::attach_power_domains(BTreeMap<String, PowerDomainHandle>)
Peripheral::sync_power_domains()
```

A handle exposes the measured-mode flag, route flag, optional voltage, range, availability, and observation time. A peripheral may use multiple roles. The generic trait rejects attachment by default; metadata cannot silently grant power capability to an unsupported model. The exact model must gate its real transport, sampling, pad, or memory behavior. Rail qualification does not invent regulator outputs or wire distinct physical nets together.

A programmable detector receives `PowerSupervisorControl` through `attach_power_supervisor`. Its exact peripheral owns register encodings, passwords, legal selector values, and unsupported register behavior. The generic control accepts only an enable state and finite ordered voltage thresholds. An identical write is a no-op. A state-changing write requires an attached sampled source, and a bounded queue refuses overflow without accepting the new request.

The accepted register configuration is immediately readable. Its detector effect starts at the next explicit sample boundary. Re-enable and reprogram delays use separate descriptor values. Host call count and wall time do not advance detector timers.

## Reset lifecycle

A latched POR observes its sense rail and a separate detector supply. Its release delay starts only while the sense threshold and detector operating qualification are satisfied. A later sense-rail fall does not rearm a latched POR. The declared detector rearm threshold does. An unqualified detector supply above the rearm threshold blocks supported execution without inventing a silicon reset event.

The first qualified POR release produces one `PowerOn` event. Later rearm produces its event at assertion; the subsequent release does not produce another. A brown-out assertion produces one `BrownOut` event and restores the descriptor's detector defaults. Its assertion and release delays use the sampled voltage history. POR and BOR use `startup_delay_ns`; an explicit external-pin or other full chip reset uses `enable_delay_ns` on the next sampled boundary.

The Machine consumes each event once. It resets modeled peripherals, then delivers `on_power_reset(PowerResetCause)` so the exact reset-cause registers record the actual event. CPU vector reload can occur later. That reload does not run another destructive peripheral reset and cannot erase the cause recorded at assertion.

A restart requires qualified execution and detector domains, a released external reset input, and a readable recorded reset-vector mapping. The preflight checks the actual linear memory span with RAM/ROM/flash precedence. A gated span leaves restart pending; an unsupported or unmapped vector table fails explicitly. QSPI is therefore a mapped-memory permission rather than an unconditional CPU power requirement. Exact gated-memory integration must be tested with the corresponding memory capability; the generic immutable base has no QSPI model-specific gate.

CPU holds do not stop the shared analog solver or circuit motor mechanics. The simulation clock continues to reach the boundaries that can restore power. GPIO voltage drive amplitudes use the current node's observed I/O rail and become high impedance when its power or reset state does not permit a drive.

## GPIO and reset threshold scope

The existing affine form remains valid:

```yaml
gpio_input_thresholds: {vil: 0.28, vih: 0.31, vil_offset_v: 0.1, vih_offset_v: 0.8}
```

The new absolute form records its nominal calibration explicitly:

```yaml
gpio_input_thresholds:
  vil_max_v: 0.8
  vih_min_v: 2.0
  nominal_io_voltage_v: 3.3
  minimum_io_voltage_v: 2.5
```

The optional minimum is an explicit qualification boundary for that GPIO mode. It is not derived by dividing nominal thresholds by a supply voltage. The actual rail need not equal the nominal calibration exactly: a solved 3.300167 V observation stays 3.300167 V. Unsupported voltage-select modes belong to the exact peripheral's capability and must be refused there. During unqualified input interpretation, the router does not manufacture an input edge; the status reports that the nominal mode is unqualified.

An external reset descriptor can select `io_domain` and, for absolute thresholds, `nominal_io_voltage_v`. It keeps its own declared pulse limits. Zero/zero limits mean sampled-level reset without an invented hardware pulse filter. An unavailable reset I/O domain holds the core without claiming that a valid low pulse occurred. GPIO-specific minimum-voltage qualifications are not copied to a reset pin without a source establishing that relationship.

## Readback and limits

`Machine::power_domain_status`, fallible `WasmSimulator.power_domain_status`, and `WorldCosimStatus.power_domains[node]` expose the same state: individual observations, execution/supervision qualification, POR latch, reset hold, pending restart, reset count/cause, accepted/applied detector settings, sample interval, and nominal threshold qualification. Older Worlds without named domains omit the new map.

Detector timing is explicitly `sample-boundary-quantized`. Voltage is observed at the configured analog boundaries; sub-interval pulses and crossings are not reconstructed. This generic contract does not model an internal regulator, switching ripple, compensation, exact startup delay distributions, efficiency, thermal behavior, or hardware-qualified input thresholds over an undeclared range. Exact part models and native-board transient acceptance remain separate evidence.
