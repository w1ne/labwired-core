# Explicit declarative register reset fixtures

A `declarative` peripheral can set `config.reset_values` to a mapping from an
existing register ID to an unsigned integer. Values are applied to that
instance before construction. The descriptor still defines each register's
address, width, access policy and side effects.

```yaml
peripherals:
  - id: uid
    type: declarative
    base_address: 0x1fff7590
    size: 12B
    config:
      path: configs/peripherals/stm32l476/uid.yaml
      reset_values: {UID0: 4097, UID1: 8193, UID2: 12289}
```

This example is a declared synthetic STM32L476 identity. It is not a measured
factory UID. The embedded descriptor is shared by native and browser builds;
no custom descriptor file or filesystem access is needed. Other instances and
the descriptor default are unchanged. Firmware writes to the read-only UID
registers remain ineffective.

Unknown register IDs, non-string IDs, non-integer values, negative values and
values outside the declared 8-, 16- or 32-bit width fail configuration. Reset
fixtures must be recorded in the system's evidence; they must not be reported
as physical readings or used to hide missing register behavior.
