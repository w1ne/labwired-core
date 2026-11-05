# LumenPnP platform models

The diagnostic job uses the Opulo LumenPnP Rev05 motherboard architecture. The upstream electrical reference is pinned at `64cf68d5b7980f095ef11913065e7f45a1e3e447`. The foundation change is based on LabWired Core `42b4a44c42814c2c36515c8f8f0b376467f3a700`. Captures record their compiled source commit and runner hash. [Electrical reference](https://github.com/opulo-inc/lumenpnp/tree/64cf68d5b7980f095ef11913065e7f45a1e3e447/src/PCB/mobo)

## Pressure acquisition

The reference's mechanical library names the pressure sensor `XGZP6857D200KPGPN33`. Its non-C protocol uses I²C address `0x6D`. OpenPnP's current configuration independently uses this address, command register `0x30`, conversion command `0x0A`, and pressure register `0x06`. [Part reference](https://github.com/opulo-inc/lumenpnp/blob/64cf68d5b7980f095ef11913065e7f45a1e3e447/src/OTS/XGZP6857D200KPGPN33.FCStd) · [OpenPnP configuration](https://github.com/opulo-inc/lumenpnp/blob/64cf68d5b7980f095ef11913065e7f45a1e3e447/openpnp/machine.xml)

The implemented sensor exposes the following behavior. [Manufacturer datasheet V2.5](https://www.endrich.com/Datenbl%C3%A4tter/Sensoren/XGZP6857D-Pressure-Sensor-V2.5.pdf)

| Feature | Model contract |
| --- | --- |
| Gauge pressure range | −100,000 to +200,000 Pa |
| Pressure output | Registers `0x06..0x08`, signed 24-bit big-endian, 32 counts/Pa |
| Temperature output | Registers `0x09..0x0A`, signed 16-bit big-endian, 256 counts/°C |
| Single acquisition | Write `0x0A` to register `0x30` |
| Conversion time | 20,000 simulated µs, using the recommended datasheet wait |
| Busy flag | Command bit 3 remains set during acquisition and clears at completion |
| Periodic acquisition | Command mode 3, sleep interval in 62.5 ms units, busy bit remains set |
| Sample publication | Output registers change when conversion completes |
| Inputs | `pressure_pa`, `temperature_c`, independently addressable by component id |
| Variant boundary | Factory OTP calibration, raw bridge mode, ADC noise and C-series protocol remain outside the model |

Conversion time advances from committed MCU cycles divided by the manifest's declared 168 MHz clock. The simulator records the use of this derived device time. Dynamic clock-tree retuning requires a corresponding model extension.

## I²C switch

The PCA9545A exposes four downstream channels and address straps A0/A1. The lower four control bits select channels after STOP. The upper four bits report the interrupt inputs and ignore writes. Simultaneously enabled sensors at the same address contribute wired-AND read data. Conversion clocks continue advancing while a channel is isolated. The model exposes reset and interrupt state through its native API. The diagnostic board ties reset inactive and uses polling. Analog pass-gate resistance, SDA/SCL rise time and interrupt GPIO routing remain outside this byte-level model. [NXP datasheet](https://www.nxp.com/docs/en/data-sheet/PCA9545A_45B_45C.pdf)

```yaml
external_devices:
  - id: vac_mux
    type: pca9545a
    connection: i2c1
    config:
      i2c_address: 0x70
  - id: vac1
    type: xgzp6857d
    connection: vac_mux
    channel: 0
  - id: vac2
    type: xgzp6857d
    connection: vac_mux
    channel: 1
```

## Motion

The TMC2226 model reuses LabWired's STEP/DIR observer with enable, direction, rising-edge count, and configurable mechanical degrees per step. Six instances represent X, Y1, Y2, Z, nozzle A and nozzle B. STM32 pins bind through the existing synchronous GPIO write hook. Direction and enable are sampled before STEP, including when a single store changes multiple pins. The binding requires actively driven pad states and preserves pulses shorter than a peripheral tick.

All six drivers have MS1 and MS2 tied to GND in the pinned PCB. The datasheet assigns this strap combination 1/8 microstepping. The diagnostic therefore uses 0.225° per rising STEP edge for a 1.8° motor. With a 20-tooth GT2 pulley, 1,600 steps/revolution divided by 40 mm/revolution yields 40 steps/mm on X/Y/Z. Nozzle axes also use 1,600 steps/revolution. Driver UART configuration, winding current regulation, torque limits, missed steps, backlash and belt compliance remain outside this digital motion model. [TMC2226 datasheet, sections 2.2 and 3.4](https://www.analog.com/media/en/technical-documentation/data-sheets/tmc2226_datasheet_rev1.10.pdf)

The official OpenPnP position/count snapshot indicates approximately 320 steps/mm on X/Y and 40 steps/mm on Z. With these pulley dimensions, the X/Y count corresponds to a 1/64 mode selected through driver configuration in the stock setup. The diagnostic uses the hardware strap default and its own declared calibration. [OpenPnP reference](https://github.com/opulo-inc/lumenpnp/blob/64cf68d5b7980f095ef11913065e7f45a1e3e447/openpnp/machine.xml)

## Vacuum plant

`vacuum.rs` integrates an isothermal, fixed-volume mass-balance approximation. With absolute nozzle pressure `p`, ambient pressure `pa`, ultimate pump pressure `pu`, pump volumetric rate `S`, leak conductance `C` and chamber volume `V`, the state equation is

```text
dp/dt = (S * (pu - p) + C * (pa - p)) / V
```

The implementation uses the exact exponential solution over each constant-input interval. Native brushed-motor shaft speed determines `S`. Energizing the valve connects the pump to the nozzle. Deenergizing it vents the nozzle. CAD contact selects the open or sealed leak conductance. Suction force equals gauge-pressure magnitude times the nozzle bore area. The geometry and mass test decide whether a package can attach or remain held.

The following pneumatic parameters are explicit engineering assumptions for the demonstration. They have no manufacturer calibration claim.

| Parameter | Value |
| --- | --- |
| Ambient absolute pressure | 101,325 Pa |
| Ultimate pump absolute pressure | 31,325 Pa |
| Effective hose/chamber volume | 10 mL |
| Pump volumetric rate at 700 rad/s | 60 mL/s |
| Open-nozzle conductance | 420 mL/s |
| Sealed leakage conductance | 0.3 mL/s |
| Vent conductance | 400 mL/s |
| Nozzle bore radius | 0.25 mm, shared with the declared CAD contract |
| Maximum pressure-dependent shaft load | 5 mN·m |

The native brushed-motor model integrates winding resistance/inductance, back EMF, torque, inertia and friction. The harness supplies its explicit electrical parameters. Plotted pump current comes from that model state. Package contact and a vacuum response are recomputed from executed GPIO motion and geometry at each coupling interval. The empty tray pocket causes the pressure failure through its open nozzle. Firmware decides the retry from sensor readings.

## Coverage boundary

These additions supply executable pressure feedback and digital motion for the functional placement job. The original board's regulators, protection devices, MOSFET transistor dynamics, connector parasitics and line-driver electrical behavior require separate electrical models and validation. The mechanical job uses declared geometry and package density assumptions. Vision alignment and the full upstream Marlin/OpenPnP workflow require their own execution evidence. The capture's coverage report records each exercised subsystem and its remaining fidelity limits.

## Regression checks

The chip tests exercise signed pressure and temperature, conversion deadlines, sample holding, periodic timing, input limits, STOP-latched channel routing, two independent fixed-address sensors, bus collisions, reset, interrupt status, and manifest channel validation. The STEP/DIR tests check alias registration, calibrated angle, enable gating, per-driver pulse and direction limits, clock configuration, latched violations, and native cycle timestamps under decimated peripheral ticks. Snapshot evidence distinguishes commanded motion from unmodeled torque and lost steps. The pneumatic tests check ambient equilibrium, sealed evacuation, open-nozzle failure, venting and time-partition invariance.
