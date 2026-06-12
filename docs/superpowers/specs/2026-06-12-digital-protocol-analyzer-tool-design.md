# Digital Protocol Analyzer Tool Design

## Purpose

LabWired playground should model the logic analyzer as an independent digital protocol instrument. Users build a circuit from validated reusable hardware blocks, wire analyzer channels to nets, run firmware, and inspect decoded protocols from the signals those probes can observe.

This spec records the product boundary for the next implementation pass. The goal is not a demo-specific UDS shortcut. The goal is a general instrument model that works across examples and makes the playground feel like a real bench.

## Product Principle

The playground has three separate concepts:

1. **Validated hardware blocks**
   Blocks such as MCUs, CAN transceivers, diagnostic testers, sensors, displays, IO-Link masters, and protocol adapters declare pins, electrical/protocol capabilities, default attributes, and simulator-device emission rules.

2. **Circuit wiring and run configuration**
   The diagram is the source of truth. If a user wires compatible blocks, the compiler derives simulator configuration from that graph. A bundled example may seed the canvas, but hidden YAML must not be the only reason the simulation works.

3. **Independent instruments**
   The logic analyzer is a separate tool block with probe pins. It only decodes what its channels are wired to. Protocol views are plugins attached to analyzer captures, not direct readers of demo firmware logs.

## Target Architecture

The architecture is:

```text
validated blocks -> diagram/net graph -> simulator trace sources -> logic analyzer capture -> protocol decoders
```

### Validated Blocks

Each reusable hardware block should expose enough metadata for both editor validation and simulator configuration:

- Pin declarations and roles.
- Required pins for the modeled behavior.
- Protocol capability metadata, such as UART TX/RX, SPI SCK/MOSI/MISO/CS, I2C SDA/SCL, CAN TX/RX, CAN_H/CAN_L.
- Default attributes for configured devices, for example UDS request ID and payload.
- Emitter rules that translate a correctly wired block graph into `external_devices` and `board_io`.

For the H563 UDS ECU example, the relevant validated blocks are:

- `nucleo-h563zi`
- `can-transceiver`
- `can-diagnostic-tool`
- `logic-analyzer`

The CAN diagnostic tester must be emitted only when its `CAN_H` or `CAN_L` pin is connected through a CAN transceiver whose `TXD` and `RXD` pins resolve to the same MCU FDCAN peripheral with correct roles.

### Diagram-Derived Simulation

The run path should prefer the current diagram-generated system YAML for playground labs. Bundled YAML is allowed as a seed or fallback, but it should not be a parallel hidden truth.

For the H563 ECU case, the diagram-generated configuration should produce a reusable external device like:

```yaml
external_devices:
  - id: "uds_tester"
    type: "can-diagnostic-tester"
    connection: "fdcan1"
    config:
      request_id: "0x7E0"
      request_data: "03 22 F1 90"
```

This is acceptable because it is derived from the wired blocks. It is not acceptable if it only exists in the bundled example manifest while the canvas blocks are visual decoration.

### Logic Analyzer

The logic analyzer remains a digital protocol tool. It has channels such as `CH0..CH3` and `GND`. It should support:

- Raw digital channel samples.
- Protocol decoder selection.
- Channel-to-net binding display.
- Capture clearing/export.
- Decoder availability based on actual wiring.

The analyzer must not decode a protocol merely because the board/example is known. It should arm a decoder only when the selected analyzer channels touch nets that can produce the required signal or trace source.

For CAN/UDS, wiring `CH0` to `CAN_H` or `CH1` to `CAN_L` should bind the analyzer to the inferred FDCAN peripheral for that bus. The UDS decoder should then filter FDCAN trace frames to that peripheral.

### Protocol Decoders

Protocol decoders are plugins over analyzer-observable data. They may consume different underlying trace forms, but the analyzer binding remains the authority for whether they can run.

Examples:

- UART decoder consumes UART byte/bit trace for the UART peripheral connected to the probed net.
- IO-Link decoder consumes IO-Link master state/trace for the probed TX/RX link.
- UDS decoder consumes CAN/FDCAN trace frames from the FDCAN peripheral inferred from the probed CAN bus.
- Future SPI/I2C decoders should follow the same pattern: infer the bus from probed nets and decode only that bus.

Decoders must not use UART marker strings as substitutes for protocol traffic.

## H563 UDS ECU Example Requirements

The H563 ECU playground example should be demoable as a normal LabWired circuit:

1. Opening the example shows at least:
   - H563 MCU board.
   - CAN transceiver.
   - UDS diagnostic tester.
   - Logic analyzer.

2. The visible wiring includes:
   - MCU FDCAN TX/RX pins to transceiver `TXD`/`RXD`.
   - Transceiver `CAN_H`/`CAN_L` to tester `CAN_H`/`CAN_L`.
   - Logic analyzer channels to the CAN bus nets.

3. Running the example:
   - Generates simulator config from the current diagram.
   - Emits the CAN diagnostic tester external device from the blocks.
   - Injects a UDS ReadDataByIdentifier request into the H563 FDCAN model.
   - Lets the ECU firmware answer through FDCAN.
   - Shows UDS request/response rows in the logic analyzer UDS decoder.

4. The decoded data includes:
   - CAN ID `0x7E0` request with service `0x22` DID `0xF190`.
   - CAN ID `0x7E8` positive response with service `0x62` DID `0xF190`.
   - VIN payload from the ECU firmware.

## Acceptance Criteria

### Universal Block Behavior

- A `can-diagnostic-tool` wired through a valid CAN transceiver to an H563 FDCAN TX/RX pair emits `type: "can-diagnostic-tester"` in generated system YAML.
- The same diagnostic tool does not emit if the transceiver is not fully wired to compatible MCU CAN TX/RX pins.
- The emitter is not specific to `stm32h5-uds-ecu`; it should be reusable for future CAN/UDS examples.

### Analyzer Behavior

- The UDS decoder is available only when analyzer channels are wired to a CAN bus net that resolves to an FDCAN peripheral.
- The UDS decoder filters trace frames to the inferred peripheral.
- The analyzer UI displays the probed channel bindings so users can see what net they are decoding.
- No decoder reads UART marker strings for CAN or UDS.

### Demoability

- The H563 ECU lab opens with a complete prewired diagram.
- Pressing Run is enough to see a UDS exchange in the analyzer.
- Removing either MCU-to-transceiver FDCAN wire prevents the generated tester and prevents the UDS decoder from arming.

### Tests

Required regression coverage:

- Board-config tests for CAN diagnostic tool emission from a valid diagram.
- Board-config negative tests for incomplete CAN transceiver binding.
- Logic analyzer binding tests proving `CAN_H`/`CAN_L` probes infer the correct FDCAN peripheral.
- Logic analyzer negative tests proving incomplete FDCAN wiring does not arm UDS.
- UDS decoder tests for ISO-TP single frame and CAN-FD single frame payload extraction.
- Playground starter diagram tests verifying MCU, transceiver, diagnostic tester, and analyzer are present and wired.

## Non-Goals For This Slice

- Analog CAN_H/CAN_L voltage waveform simulation.
- Arbitration, bit timing, error frames, termination resistance, or differential electrical modeling.
- A full multi-node CAN bus physics model.
- A new CI workflow just for this example.
- Demo-specific hardcoding in the analyzer.

Those can be future digital-twin fidelity layers, but the immediate product model is a digital protocol analyzer over validated block-derived traces.

## Implementation Notes

The current implementation already moved in this direction with:

- FDCAN trace snapshots from core/WASM.
- `can-diagnostic-tester` external device support.
- H563 UDS ECU firmware driven by an external tester instead of loopback self-injection.
- Playground UDS decoder reading FDCAN frames instead of UART markers.
- Board-config emission for CAN diagnostic tools from wired blocks.
- Analyzer-side FDCAN peripheral filtering.

Future work should continue by extracting repeated graph-walk logic into a shared board-config/instrument helper rather than duplicating per decoder. The boundary should remain: blocks produce observable simulator traces, instruments consume observations based on wiring, protocol plugins decode those observations.
