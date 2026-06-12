/**
 * compile() — ERC-gated diagram → manifest with net-derived bus binding.
 *
 * Supersedes diagram-to-config.ts (which becomes a thin wrapper here).
 */

import type { Diagram } from '../types';
import type { DiagramV2 } from '../schema';
import { migrateToV2 } from '../schema';
import { erc } from '../erc';
import type { Diagnostic } from '../erc';
import { resolveNets } from '../normalize';
import type { ResolvedNet } from '../normalize';
import { getCatalogPart } from '../catalog';
import { findPinFunction } from '../pin-mapping';
import { CHIP_YAMLS } from '../chip-yamls';
import { diag } from '../erc/diagnostic';
import { ESP32S3_IRQ_SOURCES } from './irq-ordinals';
import { parseAddr } from '../attrs';
import {
  I2C_DEVICE_ADDRESSES,
  SPI_DEVICE_TYPES,
  emitUltrasonic,
  emitPcd8544,
  emitSn74hc165,
  emitIolinkMaster,
  emitLegacyI2cDevice,
  emitSpiDevice,
  emitNeo6mGps,
  emitCanDiagnosticTool,
  emitBoardIoFromWires,
  buildSystemYaml,
} from './emitters';

export type { Diagnostic } from '../erc';

/** Result returned by compile(). */
export interface CompileResult {
  ok: boolean;
  /** Present when ok. Omitted when the board has no CHIP_YAMLS entry (e.g. esp32-s3-zero). */
  systemYaml?: string;
  chipYaml?: string;
  /** All ERC findings + compile-stage findings; errors imply !ok. */
  diagnostics: Diagnostic[];
}

// ---------------------------------------------------------------------------
// Net-derived I2C peripheral resolution
// ---------------------------------------------------------------------------

/**
 * For an i2c_device part, find the I2C peripheral that owns its SDA pin's net
 * by walking that net's members for an MCU pin with an i2c function.
 * Returns the peripheral name (e.g. "i2c0") or null if none found.
 */
function netDerivedI2cPeripheral(
  v2: DiagramV2,
  nets: ResolvedNet[],
  partId: string,
): string | null {
  // Find the SDA net: look for a net that contains partId:SDA
  const sdaKey = `${partId}:SDA`;
  const sdaNet = nets.find((n) => n.members.some((m) => `${m.part}:${m.pin}` === sdaKey));
  if (!sdaNet) return null;

  // Find the MCU part(s) in this diagram
  const mcuParts = v2.parts.filter((p) => {
    const cat = getCatalogPart(p.type);
    return cat?.deviceClass === 'mcu';
  });

  // Walk members of the SDA net for an MCU pin that declares i2c
  for (const member of sdaNet.members) {
    const isMcu = mcuParts.some((mcu) => mcu.id === member.part);
    if (!isMcu) continue;
    const boardKey = v2.board;
    const fn = findPinFunction(boardKey, member.pin, 'i2c');
    if (fn) return fn.peripheral;
  }
  return null;
}

// ---------------------------------------------------------------------------
// Main compile() function
// ---------------------------------------------------------------------------

/**
 * Compile a diagram into system + chip YAML.
 *
 * Steps:
 * 1. Run ERC; any error → {ok:false, diagnostics}.
 * 2. Migrate to v2 and resolve nets.
 * 3. Dispatch over parts by deviceClass.
 */
export function compile(input: Diagram | DiagramV2): CompileResult {
  // Step 1: ERC gate
  const ercDiags = erc(input);
  const hasError = ercDiags.some((d) => d.severity === 'error');
  if (hasError) {
    return { ok: false, diagnostics: ercDiags };
  }

  // Step 2: migrate + resolve nets
  const v2 = migrateToV2(input);
  // For emitter functions that still need v1-style diagram (wire-based), use v2.wires-compatible
  const diagramLike: Diagram = {
    board: v2.board,
    parts: v2.parts,
    wires: v2.wires,
  };
  const nets = resolveNets(v2);

  // Step 3: dispatch
  const externalDeviceEntries: string[] = [];
  const boardIoEntries: string[] = [];
  const compileDiags: Diagnostic[] = [...ercDiags];

  const processedPartIds = new Set<string>();

  for (const part of v2.parts) {
    const cat = getCatalogPart(part.type);
    const deviceClass = cat?.deviceClass;

    if (part.type === 'can-diagnostic-tool') {
      const { externalDevice } = emitCanDiagnosticTool(diagramLike, part.id);
      if (externalDevice) {
        externalDeviceEntries.push(externalDevice);
      } else {
        compileDiags.push(diag(
          'COMPILE_CAN_UNBOUND',
          'error',
          `Part "${part.id}" (can-diagnostic-tool) is not bound to any FDCAN peripheral`,
          'Wire CAN_H or CAN_L to a CAN transceiver whose TXD/RXD pins connect to MCU FDCAN pins',
          [part.id],
        ));
      }
      processedPartIds.add(part.id);
      continue;
    }

    if (deviceClass === 'mcu') {
      // MCU chip YAML selection handled below
      processedPartIds.add(part.id);
      continue;
    }

    if (deviceClass === 'passive') {
      // Passive parts emit nothing
      processedPartIds.add(part.id);
      continue;
    }

    if (deviceClass === 'board_io') {
      // board_io handled via wire-based emitter below (legacy path)
      // Also handle specific types that have dedicated emitters
      if (part.type === 'ultrasonic') {
        const { externalDevice } = emitUltrasonic(diagramLike, part.id);
        if (externalDevice) externalDeviceEntries.push(externalDevice);
        processedPartIds.add(part.id);
      }
      // Other board_io parts handled by emitBoardIoFromWires below
      continue;
    }

    if (deviceClass === 'i2c_device') {
      // Net-derived I2C binding
      const connection = netDerivedI2cPeripheral(v2, nets, part.id);
      if (!connection) {
        compileDiags.push(diag(
          'COMPILE_BUS_UNBOUND',
          'error',
          `Part "${part.id}" (${part.type}) is not bound to any I2C peripheral`,
          'Connect the SDA pin to a net that includes an MCU pin with an i2c function',
          [`${part.id}:SDA`],
        ));
        continue;
      }

      // IRQ ordinal validation for i2c devices
      const irqSource = part.attrs?.irq_source;
      if (irqSource !== undefined) {
        const irqNum = Number(irqSource);
        const expected = ESP32S3_IRQ_SOURCES[connection];
        if (expected !== undefined && irqNum !== expected) {
          compileDiags.push(diag(
            'IRQ_SOURCE_ORDINAL',
            'error',
            `Part "${part.id}" declares irq_source ${irqNum} but ${connection} requires ${expected}`,
            `Set attrs.irq_source to ${expected} (ETS_I2C_EXT0_INTR_SOURCE = ${expected} for ${connection})`,
            [part.id],
          ));
        }
      }

      // For types in the legacy I2C_DEVICE_ADDRESSES table, use legacy address
      const legacyAddress = I2C_DEVICE_ADDRESSES[part.type];
      const address = legacyAddress !== undefined
        ? legacyAddress
        : parseAddr(part.attrs?.i2c_address ? String(part.attrs.i2c_address) : undefined);

      if (part.type === 'ir') {
        // IR parts: emit type:ir with spec_path config
        const specPath = part.attrs?.spec_path;
        if (!specPath) {
          compileDiags.push(diag(
            'COMPILE_IR_NO_SPEC',
            'error',
            `Part "${part.id}" (ir) is missing required attrs.spec_path`,
            'Set attrs.spec_path to the path of the IR spec YAML (labwired_define_component)',
            [part.id],
          ));
          continue;
        }
        const addrStr = address !== undefined ? `0x${address.toString(16)}` : undefined;
        externalDeviceEntries.push(`  - id: "${part.id}"
    type: "ir"
    connection: "${connection}"${addrStr ? `
    config:
      spec_path: "${specPath}"
      i2c_address: ${addrStr}` : `
    config:
      spec_path: "${specPath}"`}`);
      } else if (legacyAddress !== undefined) {
        // Legacy I2C device (adxl345, mpu6050, bme280, oled-ssd1306)
        const { externalDevice, boardIo } = emitLegacyI2cDevice(
          part.id, part.type, connection, legacyAddress,
        );
        externalDeviceEntries.push(externalDevice);
        boardIoEntries.push(boardIo);
      } else if (address !== undefined) {
        // Generic i2c_device (pca9685, lcd1602, etc.)
        const addr = `0x${address.toString(16)}`;
        externalDeviceEntries.push(`  - id: "${part.id}"
    type: "${part.type}"
    connection: "${connection}"
    config:
      i2c_address: ${addr}`);
        boardIoEntries.push(`  - id: "${part.id}"
    kind: "i2c_device"
    peripheral: "${connection}"
    pin: 0
    signal: "input"
    active_high: true
    i2c_address: ${addr}
    device_type: "${part.type}"`);
      } else {
        // No address available — neither a legacy fixed address nor attrs.i2c_address.
        // Emit a diagnostic instead of silently dropping the device from the manifest.
        compileDiags.push(diag(
          'COMPILE_NO_ADDRESS',
          'error',
          `Part "${part.id}" (${part.type}) has no I2C address; set attrs.i2c_address (e.g. "0x40")`,
          'Set attrs.i2c_address on the part to its hardware address',
          [part.id],
        ));
      }
      processedPartIds.add(part.id);
      continue;
    }

    if (deviceClass === 'spi_device') {
      // TODO(plan-d): net-derived spi/uart binding
      if (part.type === 'pcd8544') {
        const { externalDevice, boardIo } = emitPcd8544(diagramLike, part.id);
        if (externalDevice) externalDeviceEntries.push(externalDevice);
        if (boardIo) boardIoEntries.push(boardIo);
      } else if (part.type === 'sn74hc165') {
        const { externalDevice } = emitSn74hc165(diagramLike, part.id);
        if (externalDevice) externalDeviceEntries.push(externalDevice);
      } else if (SPI_DEVICE_TYPES.has(part.type)) {
        const { externalDevice, boardIo } = emitSpiDevice(diagramLike, part.id, part.type);
        if (externalDevice) externalDeviceEntries.push(externalDevice);
        if (boardIo) boardIoEntries.push(boardIo);
      }
      // spi_device board_io (spi_device kind) handled below via wire walk for unhandled types
      processedPartIds.add(part.id);
      continue;
    }

    if (deviceClass === 'uart_device') {
      // TODO(plan-d): net-derived spi/uart binding
      if (part.type === 'iolink-master') {
        const { externalDevice } = emitIolinkMaster(diagramLike, part.id);
        if (externalDevice) externalDeviceEntries.push(externalDevice);
      } else if (part.type === 'neo6m-gps') {
        const { externalDevice, boardIo } = emitNeo6mGps(diagramLike, part.id);
        if (externalDevice) externalDeviceEntries.push(externalDevice);
        if (boardIo) boardIoEntries.push(boardIo);
      }
      processedPartIds.add(part.id);
      continue;
    }

    // Unknown device class or undefined: skip
  }

  // board_io for led/button/pwm_output/adc_input from wires (legacy path for board_io parts)
  const wireBoardIo = emitBoardIoFromWires(diagramLike);
  boardIoEntries.push(...wireBoardIo);

  const hasCompileError = compileDiags.some((d) => d.severity === 'error');
  if (hasCompileError) {
    return { ok: false, diagnostics: compileDiags };
  }

  // Chip YAML lookup (same logic as diagramToConfig)
  const chipYaml = CHIP_YAMLS[v2.board];
  const systemYaml = buildSystemYaml(externalDeviceEntries, boardIoEntries);

  return {
    ok: true,
    systemYaml,
    ...(chipYaml ? { chipYaml } : {}),
    diagnostics: compileDiags,
  };
}
