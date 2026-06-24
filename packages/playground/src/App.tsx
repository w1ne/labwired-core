import { useState, useCallback, useRef, useMemo, useEffect, type ReactNode } from 'react';
import { ProjectsModal } from './studio/ProjectsModal';
import { EmbedDialog } from './studio/EmbedDialog';
import { EmbedBadge } from './EmbedBadge';
import type { ProjectRecord } from './studio/useProjects';
import { CommandPalette } from './studio/CommandPalette';
import { useCommandPaletteItems } from './studio/useCommandPaletteItems';
import {
  RegisterGrid,
  MemoryInspector,
  InstructionTrace,
  SerialMonitor,
  SimulatorBridge,
  Ssd1306Display,
  Pcd8544Display,
  Ili9341Display,
  GpsControl,
  ThermistorControl,
  useSimulationLoop,
  LabwiredEditor,
  compileCode,
  EXAMPLE_SKETCHES,
  useEditorState,
  validateDiagram,
  validateWireConnection,
  COMPONENT_REGISTRY,
  createEmptyDiagram,
  decodeProject,
  fetchSharedProject,
  generateShareUrl,
  renderCanvasPng,
  type ShareOptions,
  isEmbedMode,
  type CompileError,
  type TraceEntry,
  type WasmModule,
  type Part,
  type Diagram,
  type BoardIoBinding,
  type ComponentState,
} from '@labwired/ui';
import { BOARD_CONFIGS, pickerBoards, type BoardConfig } from './bundled-configs';
import { resolveUiFeatures } from './uiFeatures';
import { resolveBoardForPart } from './board-resolve';
import { useUser, useClerk, useAuth } from '@clerk/clerk-react';
import { resolveRunSystemConfig } from './run-config';
import { versionRuntimeAssetUrl } from './runtime-assets';
import { StudioShell } from './studio/StudioShell';
import { ChipsProvider, useChips } from './multi-mcu/ChipsProvider';
import { ChipBridgeSync } from './multi-mcu/ChipBridgeSync';
import { useBackgroundChips } from './multi-mcu/useBackgroundChips';
import { MobileRunView } from './mobile/MobileRunView';
import { LabInfoButton } from './LabInfoButton';
import { PropertiesGate } from './multi-mcu/PropertiesGate';
import { ChipTabsBar, DrawerCloseButton } from './multi-mcu/ChipTabsBar';
import { ChipControls } from './multi-mcu/ChipControls';
import { usePerChipSims } from './multi-mcu/usePerChipSims';
import { ChipWindow } from './multi-mcu/ChipWindow';
import { ChipInspector } from './multi-mcu/ChipInspector';
import { BleAnalyzer } from './instruments/BleAnalyzer';
import { IoLinkAnalyzer } from './instruments/IoLinkAnalyzer';
import { LogicAnalyzerPanel } from './instruments/LogicAnalyzerPanel';
import { AuthPill } from './studio/AuthPill';
import { getComponentIcon } from './studio/componentIcons';
import { WatchOverlay } from './studio/WatchOverlay';
import { AccountPanel } from './studio/AccountPanel';
import { DevDrawer } from './studio/DevDrawer';
import { SimDock, type SimState as StudioSimState } from './studio/SimDock';
import { type InspectorSelection } from './studio/InspectorCard';
import { ComponentInspector } from './multi-mcu/ComponentInspector';
import { renderComponentRuntimeControl } from './multi-mcu/componentRuntimeControls';
import { PartActions } from './multi-mcu/PartActions';
import { type PaletteComponent, type PaletteCategory } from './studio/PaletteDrawer';
import { trackUsage } from './usage';
import { syncSensorAttributeToSimulator } from './sensor-attribute-sync';

type WorkspaceKind = 'diagram' | 'source';
type ActiveSimulationConfig = {
  systemYaml: string;
  chipYaml: string;
  firmware: Uint8Array;
  /** Firmware-runtime quirks; propagated from BoardConfig.quirks.
   *  - `esp32-arduino`: preset-PC install with hardcoded thunk addresses for a specific firmware build.
   *  - `arduino-esp32-autodiscover`: resolves thunk PCs from the firmware
   *    ELF symbol table — works for any GxEPD2 sketch (labwired-ereader). */
  quirks?: 'esp32-arduino' | 'arduino-esp32-autodiscover';
  /** Optional pre-warmed snapshot URL; applied right after firmware load. */
  bootSnapshotUrl?: string;
};

let partCounter = 0;
function nextPartId(type: string): string {
  return `${type}${++partCounter}`;
}

function getWorkspaceStorageKey(boardId: string, kind: WorkspaceKind): string {
  return `labwired-${kind}:${boardId}`;
}

function hasSavedWorkspace(boardId: string): boolean {
  const config = BOARD_CONFIGS.find((c) => c.boardId === boardId);
  if (config?.kind === 'lab') return false;
  return !!(
    localStorage.getItem(getWorkspaceStorageKey(boardId, 'diagram'))
    || localStorage.getItem(getWorkspaceStorageKey(boardId, 'source'))
  );
}

function parseDiagramMcuPin(pinLabel: string): { peripheral: string; pin: number } | null {
  const stm = pinLabel.match(/^P([A-Z])(\d+)$/i);
  if (!stm) return null;
  return {
    peripheral: `gpio${stm[1].toLowerCase()}`,
    pin: parseInt(stm[2], 10),
  };
}

function resolveBindingPartId(diagram: Diagram, binding: BoardIoBinding): string {
  if (diagram.parts.some((part) => part.id === binding.id)) {
    return binding.id;
  }

  for (const wire of diagram.wires) {
    const mcuEnd = wire.from.part === 'mcu' ? wire.from : wire.to.part === 'mcu' ? wire.to : null;
    const partEnd = mcuEnd === wire.from ? wire.to : mcuEnd === wire.to ? wire.from : null;
    if (!mcuEnd || !partEnd) continue;

    const parsedPin = parseDiagramMcuPin(mcuEnd.pin);
    if (!parsedPin) continue;
    if (parsedPin.peripheral !== binding.peripheral || parsedPin.pin !== binding.pin) continue;

    const part = diagram.parts.find((candidate) => candidate.id === partEnd.part);
    const def = part ? COMPONENT_REGISTRY.get(part.type) : null;
    if (def?.boardIoKind === binding.kind) {
      return partEnd.part;
    }
  }

  return binding.id;
}

export const LAB_NOTES: Record<string, string> = {
  'ntc-thermistor-lab':
    'NTC 3950 thermistor on the STM32F103 ADC. The Steinhart–Hart equation turns raw ADC counts into °C.\nTry: drag the temperature slider and watch the ADC count and computed temperature track it.',
  'neo6m-gps-lab':
    'NEO-6M GPS over UART. Real NMEA sentences stream in and are parsed live.\nTry: Run and watch live position and satellite data decode.',
  'quectel-bg770a-lab':
    'Quectel BG770A LTE-M / NB-IoT modem over UART, with a byte-exact AT-command surface (MQTT/HTTP/GPS state machines).\nTry: Run and watch the firmware drive the modem through its AT sequence.',
  'ssd1306-hello-lab':
    'SSD1306 128×64 OLED over I²C. The firmware draws into a framebuffer the panel renders pixel-for-pixel.\nTry: Run and watch the display paint.',
  'nokia5110-invaders-lab':
    'Nokia 5110 (PCD8544) LCD + ultrasonic sensor on the STM32L476 — a tiny Space-Invaders-style demo.\nTry: Run, then drag the distance sensor to steer.',
  'al2205-iolink-dido':
    'An IO-Link digital-input device (AL2205 profile). Speaks the IO-Link wake-up and process-data cycle.\nTry: open the IO-Link analyzer and Run to watch the master/device exchange.',
  'stm32h5-uds-ecu':
    'A minimal automotive diagnostic ECU on the STM32H5, answering UDS (ISO-14229) requests over FDCAN.\nTry: open the UDS analyzer and Run to send services and read responses.',
  'bme280-weather-lab':
    'BME280 temperature / humidity / pressure sensor over I²C.\nTry: Run and watch the three environmental readings update.',
  'ili9341-tft-lab':
    'ILI9341 240×320 RGB565 TFT over SPI. The firmware pushes a live color framebuffer.\nTry: Run and watch the panel render in color.',
  'labwired-ereader':
    'An ESP32 e-reader sketch (Arduino .ino, unmodified) driving an e-paper page, ROM-booted.\nTry: Run and page through the reader.',
  'max31855-thermocouple-lab':
    'MAX31855 K-type thermocouple amplifier over SPI, with cold-junction compensation.\nTry: drag the temperature input and watch the converted reading.',
  'mpu6050-sensor-lab':
    'MPU6050 6-axis IMU (accelerometer + gyro) over I²C.\nTry: Run and watch the motion axes update.',
  'vl53l1x-tof-lab':
    'VL53L1X time-of-flight distance sensor over I²C.\nTry: drag the distance and watch the ranging value follow.',
  'adxl345-sensor-lab':
    'ADXL345 3-axis accelerometer over I²C.\nTry: Run and watch the acceleration axes update.',
  'nrf52840-ble-lab':
    'Two nRF52840s on one canvas — a sensor node and a collector — talking over simulated BLE (no wires; they meet on the air).\nTry: Run and watch the sensor advertise and the collector receive.',
  'nrf52840-proximity-lab':
    'An nRF52840 reading a proximity sensor and reporting over BLE.\nTry: Run and watch proximity events broadcast.',
};

function withLabNote(config: BoardConfig, diagram: Diagram): Diagram {
  if (config.kind !== 'lab' || config.hidden) return diagram;
  const text = LAB_NOTES[config.boardId];
  if (!text) return diagram;
  // Place the note just above the MCU (seeded at x:100, y:100), aligned to its
  // horizontal span. This empty band is visible in both the desktop editor view
  // (which frames on the circuit and clips far-out content) and the mobile
  // fit-to-all view. Keeping it close above — rather than far above or far to
  // the side — keeps it on-screen on desktop without collapsing the mobile
  // fit-zoom. diagramBounds fits all parts, so negative y is safe.
  const note: Part = { id: 'note', type: 'note', x: 100, y: -8, rotate: 0, attrs: { text } };
  return { ...diagram, parts: [note, ...diagram.parts] };
}

export function makeStarterDiagram(config: BoardConfig): Diagram {
  const mcu: Part = {
    id: 'mcu',
    type: config.mcuComponentType,
    x: 100,
    y: 100,
    rotate: 0,
    attrs: {},
  };

  if (config.boardId === 'nrf52840-ble-lab') {
    // Two nRF52840s on one canvas. Each MCU part carries attrs.boardId so the
    // resolver gives each its own firmware (sensor TX vs collector RX). They
    // talk over the shared virtual air, not copper — hence no wires. The first
    // part keeps id 'mcu' so the default foreground (foregroundPartId ?? 'mcu')
    // targets it on load.
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        { id: 'mcu', type: config.mcuComponentType, x: 100, y: 160, rotate: 0,
          attrs: { boardId: 'nrf52840-ble-sensor' } },
        { id: 'mcu-collector', type: config.mcuComponentType, x: 560, y: 160, rotate: 0,
          attrs: { boardId: 'nrf52840-ble-collector' } },
      ],
      wires: [],
    });
  }

  if (config.boardId === 'stm32f103-blinky') {
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'led_pa5', type: 'led', x: 390, y: 90, rotate: 0, scale: 1.5, attrs: { color: 'green' } },
      ],
      wires: [
        {
          from: { part: 'mcu', pin: 'PA5' },
          to: { part: 'led_pa5', pin: 'A' },
          color: '#27c93f',
        },
      ],
    });
  }

  // -------- I²C labs (oled, sensors): all share PB6 SCL / PB7 SDA on I2C1 --------

  if (config.boardId === 'ssd1306-hello-lab') {
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'oled', type: 'oled-ssd1306', x: 540, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'oled', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'oled', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PB6' }, to: { part: 'oled', pin: 'SCL' }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PB7' }, to: { part: 'oled', pin: 'SDA' }, color: '#B07BFF' },
      ],
    });
  }

  if (config.boardId === 'nokia5110-invaders-lab') {
    // STM32L476 Breakout: Nokia 5110 (PCD8544 SPI1) + HC-SR04. Part ids match
    // the lab's external_device ids ('lcd', 'dist') so the framebuffer fetch
    // and distance setter resolve.
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'lcd', type: 'pcd8544', x: 500, y: 60, rotate: 0, attrs: {} },
        { id: 'dist', type: 'ultrasonic', x: 500, y: 280, rotate: 0, attrs: { distance: '100' } },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'lcd', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'lcd', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'lcd', pin: 'CLK' }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PA7' }, to: { part: 'lcd', pin: 'DIN' }, color: '#B07BFF' },
        { from: { part: 'mcu', pin: 'PC7' }, to: { part: 'lcd', pin: 'DC' }, color: '#3DD68C' },
        { from: { part: 'mcu', pin: 'PB6' }, to: { part: 'lcd', pin: 'CE' }, color: '#FFD166' },
        { from: { part: 'mcu', pin: 'PA9' }, to: { part: 'lcd', pin: 'RST' }, color: '#EF476F' },
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'dist', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'dist', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PA8' }, to: { part: 'dist', pin: 'TRIG' }, color: '#06D6A0' },
        { from: { part: 'mcu', pin: 'PB10' }, to: { part: 'dist', pin: 'ECHO' }, color: '#118AB2' },
      ],
    });
  }

  if (config.boardId === 'nrf52840-proximity-lab') {
    // nRF52840 + HC-SR04 ultrasonic proximity, all on P0. The 'ultrasonic'
    // part id matches the lab's external_device id so the distance setter
    // resolves; the LED on P0.06 lights when the firmware raises ALARM.
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        // Start out of range (100 cm) so the ALARM LED is OFF until you drag the
        // Distance slider down past the firmware's 50 cm threshold — the toggle
        // is the point of the lab.
        { id: 'ultrasonic', type: 'ultrasonic', x: 520, y: 80, rotate: 0, attrs: { distance: '100' } },
        { id: 'alarm_led', type: 'led', x: 520, y: 300, rotate: 0, scale: 1.5, attrs: { color: 'red' } },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VDD' }, to: { part: 'ultrasonic', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'ultrasonic', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu', pin: 'P0.04' }, to: { part: 'ultrasonic', pin: 'TRIG' }, color: '#06D6A0' },
        { from: { part: 'mcu', pin: 'P0.05' }, to: { part: 'ultrasonic', pin: 'ECHO' }, color: '#118AB2' },
        { from: { part: 'mcu', pin: 'P0.06' }, to: { part: 'alarm_led', pin: 'A' }, color: '#EF476F' },
      ],
    });
  }

  if (config.boardId === 'al2205-iolink-dido') {
    // STM32L476 IO-Link DI device. Part ids match the lab's external_device
    // ids ('di_shifter', 'iolink_master') so the 74HC165 input toggles and the
    // IO-Link master state/PD readout resolve against the bridge.
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'di_shifter', type: 'sn74hc165', x: 520, y: 70, rotate: 0, attrs: {} },
        { id: 'iolink_master', type: 'iolink-master', x: 520, y: 300, rotate: 0, attrs: {} },
        // Logic Analyzer pre-probed on the IO-Link line and pre-armed to the
        // IO-Link decoder, so opening the tool immediately shows the cyclic
        // process data — and toggling the 74HC165 inputs highlights the PD
        // change live (the "CHG" badge). No manual wiring needed for the demo.
        { id: 'iolink_probe', type: 'logic-analyzer', x: 760, y: 300, rotate: 0, attrs: { decoder: 'iolink' } },
      ],
      wires: [
        // 74HC165 digital-input shift register on SPI1 (CS = PA4).
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'di_shifter', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'di_shifter', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'di_shifter', pin: 'CLK' }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PA6' }, to: { part: 'di_shifter', pin: 'QH' }, color: '#B07BFF' },
        { from: { part: 'mcu', pin: 'PA4' }, to: { part: 'di_shifter', pin: 'SH_LD' }, color: '#FFD166' },
        // IO-Link master peer on USART2 (PA2 TX, PA3 RX).
        { from: { part: 'mcu', pin: 'PA2' }, to: { part: 'iolink_master', pin: 'RX' }, color: '#06D6A0' },
        { from: { part: 'mcu', pin: 'PA3' }, to: { part: 'iolink_master', pin: 'TX' }, color: '#118AB2' },
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'iolink_master', pin: 'L+' }, color: '#FF6B6B' },
        // Logic Analyzer probes: CH0 on the master TX, CH1 on the master RX.
        { from: { part: 'iolink_probe', pin: 'CH0' }, to: { part: 'iolink_master', pin: 'TX' }, color: '#F5B642' },
        { from: { part: 'iolink_probe', pin: 'CH1' }, to: { part: 'iolink_master', pin: 'RX' }, color: '#F5B642' },
      ],
    });
  }

  if (config.boardId === 'stm32h5-uds-ecu') {
    // STM32H563 UDS ECU with a reusable diagnostic tester on the CAN bus.
    // The logic analyzer decodes the simulator's FDCAN frame trace for the
    // probed CAN_H/CAN_L net.
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'can_xcvr', type: 'can-transceiver', x: 500, y: 210, rotate: 0, attrs: {} },
        { id: 'uds_tester', type: 'can-diagnostic-tool', x: 680, y: 205, rotate: 0, attrs: {} },
        { id: 'uds_probe', type: 'logic-analyzer', x: 880, y: 196, rotate: 0, attrs: { decoder: 'uds' } },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PD1' }, to: { part: 'can_xcvr', pin: 'TXD' }, color: '#06D6A0' },
        { from: { part: 'mcu', pin: 'PD0' }, to: { part: 'can_xcvr', pin: 'RXD' }, color: '#118AB2' },
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'can_xcvr', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'can_xcvr', pin: 'GND' }, color: '#888888' },
        { from: { part: 'can_xcvr', pin: 'CAN_H' }, to: { part: 'uds_tester', pin: 'CAN_H' }, color: '#06D6A0' },
        { from: { part: 'can_xcvr', pin: 'CAN_L' }, to: { part: 'uds_tester', pin: 'CAN_L' }, color: '#118AB2' },
        { from: { part: 'can_xcvr', pin: 'GND' }, to: { part: 'uds_tester', pin: 'GND' }, color: '#888888' },
        { from: { part: 'uds_probe', pin: 'CH0' }, to: { part: 'uds_tester', pin: 'CAN_H' }, color: '#F5B642' },
        { from: { part: 'uds_probe', pin: 'CH1' }, to: { part: 'uds_tester', pin: 'CAN_L' }, color: '#F5B642' },
        { from: { part: 'uds_probe', pin: 'GND' }, to: { part: 'uds_tester', pin: 'GND' }, color: '#888888' },
      ],
    });
  }

  if (config.boardId === 'f103-uds-ecu' || config.boardId === 'f103-uds-ecu-broken') {
    // STM32F103 ECU on the bxCAN (classical CAN) model. The logic analyzer
    // decodes the bxCAN frame trace; bxCAN runs in internal loopback so the
    // single node plays both tester and ECU (PA12 = CAN_TX, PA11 = CAN_RX).
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'can_xcvr', type: 'can-transceiver', x: 500, y: 210, rotate: 0, attrs: {} },
        { id: 'uds_tester', type: 'can-diagnostic-tool', x: 680, y: 205, rotate: 0, attrs: {} },
        { id: 'uds_probe', type: 'logic-analyzer', x: 880, y: 196, rotate: 0, attrs: { decoder: 'uds' } },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PA12' }, to: { part: 'can_xcvr', pin: 'TXD' }, color: '#06D6A0' },
        { from: { part: 'mcu', pin: 'PA11' }, to: { part: 'can_xcvr', pin: 'RXD' }, color: '#118AB2' },
        { from: { part: 'mcu', pin: '5V' }, to: { part: 'can_xcvr', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'can_xcvr', pin: 'GND' }, color: '#888888' },
        { from: { part: 'can_xcvr', pin: 'CAN_H' }, to: { part: 'uds_tester', pin: 'CAN_H' }, color: '#06D6A0' },
        { from: { part: 'can_xcvr', pin: 'CAN_L' }, to: { part: 'uds_tester', pin: 'CAN_L' }, color: '#118AB2' },
        { from: { part: 'can_xcvr', pin: 'GND' }, to: { part: 'uds_tester', pin: 'GND' }, color: '#888888' },
        { from: { part: 'uds_probe', pin: 'CH0' }, to: { part: 'uds_tester', pin: 'CAN_H' }, color: '#F5B642' },
        { from: { part: 'uds_probe', pin: 'CH1' }, to: { part: 'uds_tester', pin: 'CAN_L' }, color: '#F5B642' },
        { from: { part: 'uds_probe', pin: 'GND' }, to: { part: 'uds_tester', pin: 'GND' }, color: '#888888' },
      ],
    });
  }

  if (config.boardId === 'mpu6050-sensor-lab') {
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'mpu6050', type: 'mpu6050', x: 540, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'mpu6050', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'mpu6050', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PB6' }, to: { part: 'mpu6050', pin: 'SCL' }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PB7' }, to: { part: 'mpu6050', pin: 'SDA' }, color: '#B07BFF' },
      ],
    });
  }

  if (config.boardId === 'adxl345-sensor-lab') {
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'adxl345', type: 'adxl345', x: 540, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'adxl345', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'adxl345', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PB6' }, to: { part: 'adxl345', pin: 'SCL' }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PB7' }, to: { part: 'adxl345', pin: 'SDA' }, color: '#B07BFF' },
      ],
    });
  }

  if (config.boardId === 'bme280-weather-lab') {
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'bme280', type: 'bme280', x: 540, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'bme280', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'bme280', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PB6' }, to: { part: 'bme280', pin: 'SCL' }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PB7' }, to: { part: 'bme280', pin: 'SDA' }, color: '#B07BFF' },
      ],
    });
  }

  // -------- SPI labs --------

  if (config.boardId === 'ili9341-tft-lab') {
    // ILI9341 sim ignores D/C (state machine over command boundaries), but
    // real hardware needs it — wire to PB0 so the same diagram is honest for
    // both. RESET wired to PB1; LED backlight tied to VCC.
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'tft', type: 'ili9341', x: 540, y: 60, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'tft', pin: 'VCC'   }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'tft', pin: 'GND'   }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PA4' }, to: { part: 'tft', pin: 'CS'    }, color: '#3DD68C' },
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'tft', pin: 'SCK'   }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PA7' }, to: { part: 'tft', pin: 'MOSI'  }, color: '#B07BFF' },
        { from: { part: 'mcu', pin: 'PB0' }, to: { part: 'tft', pin: 'DC'    }, color: '#5B9DFF' },
        { from: { part: 'mcu', pin: 'PB1' }, to: { part: 'tft', pin: 'RESET' }, color: '#F5B642' },
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'tft', pin: 'LED'   }, color: '#FFE680' },
      ],
    });
  }

  if (config.boardId === 'max31855-thermocouple-lab') {
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'tc1', type: 'max31855', x: 540, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'tc1', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'tc1', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PA4' }, to: { part: 'tc1', pin: 'CS'  }, color: '#3DD68C' },
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'tc1', pin: 'SCK' }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PA6' }, to: { part: 'tc1', pin: 'DO'  }, color: '#B07BFF' },
      ],
    });
  }

  // -------- UART --------

  if (config.boardId === 'neo6m-gps-lab') {
    // STM32 TX → GPS RX, GPS TX → STM32 RX (crossover).
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'gps', type: 'neo6m-gps', x: 540, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC'  }, to: { part: 'gps', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND'  }, to: { part: 'gps', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PA9'  }, to: { part: 'gps', pin: 'RX'  }, color: '#B07BFF' },
        { from: { part: 'mcu', pin: 'PA10' }, to: { part: 'gps', pin: 'TX'  }, color: '#5BD8FF' },
      ],
    });
  }

  if (config.boardId === 'quectel-bg770a-lab') {
    // STM32 USART1 ↔ modem (PA9 TX → modem RX, PA10 RX ← modem TX).
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'modem', type: 'bg770a-cellular', x: 540, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu',   pin: 'VCC'  }, to: { part: 'modem', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu',   pin: 'GND'  }, to: { part: 'modem', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu',   pin: 'PA9'  }, to: { part: 'modem', pin: 'RX'  }, color: '#B07BFF' },
        { from: { part: 'mcu',   pin: 'PA10' }, to: { part: 'modem', pin: 'TX'  }, color: '#5BD8FF' },
      ],
    });
  }

  // -------- Analog (ADC) --------

  if (config.boardId === 'ntc-thermistor-lab') {
    // NTC voltage divider sits between VCC and GND; tap into ADC1 ch0 on PA0.
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'ntc', type: 'ntc-thermistor', x: 540, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PA0' }, to: { part: 'ntc', pin: 'A' }, color: '#F5B642' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'ntc', pin: 'B' }, color: '#888888' },
      ],
    });
  }

  if (config.boardId === 'epaper-tricolor-lab') {
    // STM32F103 driving the Waveshare 2.9" SSD1680 tri-color panel.
    // Pin map matches the firmware (examples/epaper-tricolor-lab/src/main.rs)
    // AND a real NUCLEO-F103RB wiring of the panel — same ELF runs in both.
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'epaper', type: 'ssd1680_tricolor_290', x: 540, y: 40, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' },  to: { part: 'epaper', pin: 'VCC'  }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' },  to: { part: 'epaper', pin: 'GND'  }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PA7' },  to: { part: 'epaper', pin: 'DIN'  }, color: '#B07BFF' },
        { from: { part: 'mcu', pin: 'PA5' },  to: { part: 'epaper', pin: 'CLK'  }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PA4' },  to: { part: 'epaper', pin: 'CS'   }, color: '#3DD68C' },
        { from: { part: 'mcu', pin: 'PB0' },  to: { part: 'epaper', pin: 'DC'   }, color: '#5B9DFF' },
        { from: { part: 'mcu', pin: 'PA9' },  to: { part: 'epaper', pin: 'RST'  }, color: '#F5B642' },
        { from: { part: 'mcu', pin: 'PC7' },  to: { part: 'epaper', pin: 'BUSY' }, color: '#FFE680' },
      ],
    });
  }

  if (config.boardId === 'esp32-epaper-lab' || config.boardId === 'labwired-ereader') {
    // ESP32-WROOM-32 driving a Waveshare 2.9" tri-color e-paper via VSPI.
    // Same VSPI wiring for both: the Rust no_std `esp32-epaper-lab` firmware
    // and the Arduino `labwired-ereader` sketch both drive the same physical
    // pinout (BUSY=GPIO4 / RST=GPIO16 / DC=GPIO17 / CS=GPIO5 / SCK=GPIO18 /
    // MOSI=GPIO23) so the diagram seed is identical.
    //
    // Panel type differs by firmware: Rust no_std drives the SSD1680
    // controller directly, while the Arduino sketch uses GxEPD2 which
    // emits UC8151D opcodes — autodiscover quirks attach a UC8151D model
    // and we render from that panel's framebuffer.
    //
    // `panelScale` from BoardConfig — the panel face renders at 144×48
    // SVG units; without an upscale 12-px font glyphs collapse to ~4
    // screen pixels and the rendered text is unreadable.
    const panelScale = config.panelScale ?? 1;
    const panelType =
      config.boardId === 'labwired-ereader'
        ? 'uc8151d_tricolor_290'
        : 'ssd1680_tricolor_290';
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'epaper', type: panelType, x: 600, y: 20, rotate: 0, scale: panelScale, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: '3V3' },     to: { part: 'epaper', pin: 'VCC'  }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' },     to: { part: 'epaper', pin: 'GND'  }, color: '#888888' },
        { from: { part: 'mcu', pin: 'GPIO23' },  to: { part: 'epaper', pin: 'DIN'  }, color: '#B07BFF' },
        { from: { part: 'mcu', pin: 'GPIO18' },  to: { part: 'epaper', pin: 'CLK'  }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'GPIO5'  },  to: { part: 'epaper', pin: 'CS'   }, color: '#3DD68C' },
        { from: { part: 'mcu', pin: 'GPIO17' },  to: { part: 'epaper', pin: 'DC'   }, color: '#5B9DFF' },
        { from: { part: 'mcu', pin: 'GPIO16' },  to: { part: 'epaper', pin: 'RST'  }, color: '#F5B642' },
        { from: { part: 'mcu', pin: 'GPIO4'  },  to: { part: 'epaper', pin: 'BUSY' }, color: '#FFE680' },
      ],
    });
  }

  if (config.boardId === 'nucleo-f401re') {
    return withLabNote(config, {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'led2_pa5', type: 'led', x: 390, y: 90, rotate: 0, scale: 1.5, attrs: { color: 'green' } },
        { id: 'button_user_pc13', type: 'button', x: 300, y: -20, rotate: 0, scale: 1.35, attrs: {} },
      ],
      wires: [
        {
          from: { part: 'mcu', pin: 'PA5' },
          to: { part: 'led2_pa5', pin: 'A' },
          color: '#27c93f',
        },
        {
          from: { part: 'mcu', pin: 'PC13' },
          to: { part: 'button_user_pc13', pin: '1' },
          color: '#569cd6',
        },
      ],
    });
  }

  return withLabNote(config, {
    ...createEmptyDiagram(config.chipId),
    parts: [mcu],
    wires: [],
  });
}

function getDefaultSource(config: BoardConfig): string {
  if (config.boardId === 'nucleo-f401re') {
    return EXAMPLE_SKETCHES.find((sketch) => sketch.name === 'Button + LED')?.source ?? EXAMPLE_SKETCHES[0].source;
  }
  return EXAMPLE_SKETCHES.find((sketch) => sketch.name === 'Blink')?.source ?? EXAMPLE_SKETCHES[0].source;
}

/**
 * Whether a circuit will actually run when its share is opened.
 *
 * Runnability is grounded in ONE fact: will bootable firmware exist on open?
 * It is NOT a property of the editor text. The signals, in order:
 *   1. The board ships a prebuilt demo ELF (`demoFirmwarePath`) — boots directly.
 *   2. It is a curated lab (`kind: 'lab'`) — labs always ship prebuilt firmware.
 *      Multi-chip labs (e.g. the nRF52840 BLE sensor+collector) load firmware
 *      PER CHIP, so the top-level `demoFirmwarePath` is absent even though the
 *      lab runs — hence the explicit `kind` check, not a firmware-path check.
 *   3. Otherwise it's a bare board: runnable only once the user has authored
 *      code (source differs from the untouched default template). Sharing an
 *      untouched default on a bare board yields a link that can't run — the
 *      original dead-proximity/sensor-share failure.
 *
 * Earlier this inferred (1)+(3) only and used the source-vs-default text as the
 * lab signal too, which false-warned "no code" on every curated example whose
 * firmware doesn't come from the editor (all multi-chip labs). The lab is the
 * fundamental unit of runnability here, so it gets a first-class check.
 */
export function sharedCircuitIsRunnable(config: BoardConfig, source: string): boolean {
  if (config.demoFirmwarePath) return true;
  if (config.kind === 'lab') return true;
  return source.trim() !== getDefaultSource(config).trim();
}

function loadBoardWorkspace(config: BoardConfig): { diagram: Diagram; source: string } {
  if (config.kind === 'lab') {
    return {
      diagram: makeStarterDiagram(config),
      source: getDefaultSource(config),
    };
  }

  const savedDiagram = localStorage.getItem(getWorkspaceStorageKey(config.boardId, 'diagram'));
  const savedSource = localStorage.getItem(getWorkspaceStorageKey(config.boardId, 'source'));

  let diagram = makeStarterDiagram(config);
  if (savedDiagram) {
    try {
      const parsed = JSON.parse(savedDiagram) as Diagram;
      const nonMcuParts = (parsed.parts ?? []).filter((p) => p.id !== 'mcu');
      // Fall back to the starter when the saved diagram has been emptied — visitors should
      // always land on a running-ready circuit, not a blank canvas.
      diagram = nonMcuParts.length === 0 ? makeStarterDiagram(config) : parsed;
      // Migrate stale saves: ereader's 'epaper' part used to be typed as
      // ssd1680_tricolor_290 before the UC8151D split. The firmware emits
      // GxEPD2's UC8151D opcode stream so the SSD1680 buffer never gets
      // written; the saved (stale) type made the panel render solid red.
      // Discard the saved diagram in that case so the fresh UC8151D
      // starter takes over.
      if (
        config.boardId === 'labwired-ereader' &&
        diagram.parts?.some((p) => p.id === 'epaper' && p.type === 'ssd1680_tricolor_290')
      ) {
        diagram = makeStarterDiagram(config);
      }
    } catch {
      diagram = makeStarterDiagram(config);
    }
  }

  return {
    diagram,
    source: savedSource ?? getDefaultSource(config),
  };
}

// First-visit default: a Blue Pill with one blinking LED — the canonical
// embedded "hello world". Simple, no wiring errors possible, Run shows it
// blinking immediately. Falls back to the first config if this id ever moves.
const DEFAULT_BOARD =
  BOARD_CONFIGS.find((c) => c.boardId === 'stm32f103-blinky') ?? BOARD_CONFIGS[0];
const DEMO_AUTOSTART_KEY = 'labwired-demo-autostart-v1';

// When a lab has no firmware of its own (no compiler on prod, no demoFirmwarePath
// — e.g. an agent/shared bare-board diagram), fall back to a curated example's
// pre-built ELF that matches the MCU, so the lab still runs. Binaries are only
// ever sourced from examples, never attached to bare board configs.
function pickFallbackDemoFirmware(diagram: Diagram): string | null {
  const base = import.meta.env.BASE_URL;
  const fw = (file: string) => `${base}wasm/${file}`;
  const board = String(diagram.board ?? '').toLowerCase();
  if (board.includes('nrf52840')) return fw('demo-nrf52840-proximity.elf');
  if (board.includes('l476') || board.startsWith('stm32l4')) return fw('demo-stm32l476-blink.elf');
  if (board.includes('f401') || board.startsWith('stm32f4')) return fw('demo-nucleo-f401.elf');
  if (board.includes('f103') || board.startsWith('stm32f1')) return fw('demo-blinky.elf');
  return null;
}

export function resolveSharedBoardConfig(diagram: Diagram): BoardConfig | null {
  const boardId = diagram.board;
  return (
    BOARD_CONFIGS.find((config) => config.kind !== 'lab' && config.boardId === boardId)
    ?? BOARD_CONFIGS.find((config) => config.kind !== 'lab' && config.chipId === boardId)
    ?? BOARD_CONFIGS.find((config) => config.boardId === boardId)
    ?? null
  );
}

function isGenericSharedMcuType(type: string, diagramBoard: string, config: BoardConfig): boolean {
  return type === 'mcu' || type === diagramBoard || type === config.chipId || type === 'stm32';
}

export function prepareSharedProjectForPlayground(diagram: Diagram): { board: BoardConfig; diagram: Diagram } | null {
  const board = resolveSharedBoardConfig(diagram);
  if (!board) return null;
  const parts = diagram.parts.map((part) => {
    if (part.id !== 'mcu' || !isGenericSharedMcuType(part.type, diagram.board, board)) {
      return part;
    }
    return { ...part, type: board.mcuComponentType };
  });
  return {
    board,
    diagram: {
      ...diagram,
      board: board.chipId,
      parts,
    },
  };
}

const PALETTE_CATEGORY: Record<string, PaletteCategory> = {
  adxl345: 'i2c',
  bme280: 'i2c',
  ili9341: 'spi',
  max31855: 'spi',
  mpu6050: 'i2c',
  'oled-ssd1306': 'i2c',
  'neo6m-gps': 'uart',
  'bg770a-cellular': 'uart',
  'ntc-thermistor': 'analog',
  lcd1602: 'i2c',
  dht22: 'misc',
  led: 'gpio',
  button: 'gpio',
  'rgb-led': 'gpio',
  buzzer: 'gpio',
  'seven-segment': 'gpio',
  'led-matrix': 'gpio',
  neopixel: 'gpio',
  servo: 'gpio',
  'motor-driver-l293d': 'gpio',
  potentiometer: 'analog',
  ldr: 'analog',
  ultrasonic: 'misc',
  'pir-sensor': 'gpio',
  keypad: 'gpio',
  'slide-switch': 'gpio',
  'dip-switch': 'gpio',
  'rotary-encoder': 'gpio',
  resistor: 'misc',
  capacitor: 'misc',
  diode: 'misc',
  transistor: 'misc',
  'shift-register-74hc595': 'misc',
  'logic-analyzer': 'tools',
};

// Wall-clock time the MCU has been running, mm:ss.
function formatRuntime(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const mm = String(Math.floor(totalSeconds / 60)).padStart(2, '0');
  const ss = String(totalSeconds % 60).padStart(2, '0');
  return `${mm}:${ss}`;
}

// Resolve an MCU diagram part to its BoardConfig (see board-resolve.ts).
function mcuBoardForPart(
  part: { id: string; type: string; attrs?: Record<string, unknown> | null } | undefined,
  primaryBoard: BoardConfig,
): BoardConfig | null {
  if (!part) return null;
  return resolveBoardForPart(part, primaryBoard, BOARD_CONFIGS);
}

function EmptyTabState({ label }: { label: string }) {
  return (
    <div className="h-full flex items-center justify-center px-6">
      <div className="text-fg-tertiary text-[12px] text-center max-w-[28ch]">
        <div className="text-fg-secondary text-[13px] mb-1">▶ Idle</div>
        {label}
      </div>
    </div>
  );
}

export function App() {
  // ?watch=<sessionId> short-circuits the entire playground into a read-only
  // overlay that mirrors an agent-driven session via WebSocket.
  const watchSessionId =
    typeof window !== 'undefined'
      ? (() => {
          const id = new URLSearchParams(window.location.search).get('watch');
          return id && /^[A-Za-z0-9_-]{4,64}$/.test(id) ? id : null;
        })()
      : null;
  if (watchSessionId) return <WatchOverlay sessionId={watchSessionId} />;

  const [wasmModule, setWasmModule] = useState<WasmModule | null>(null);
  const [bridge, setBridge] = useState<SimulatorBridge | null>(null);
  const [activeSimulationConfig, setActiveSimulationConfig] = useState<ActiveSimulationConfig | null>(null);
  const [loading, setLoading] = useState(false);
  const [, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const traceRef = useRef<TraceEntry[]>([]);
  const [traceEntries, setTraceEntries] = useState<TraceEntry[]>([]);
  const [canvasValidationMessage, setCanvasValidationMessage] = useState<string | null>(null);
  const [invalidPins, setInvalidPins] = useState<Array<{ part: string; pin: string }>>([]);

  // Board selection (from catalog + bundled configs)
  const [selectedBoard, setSelectedBoard] = useState<BoardConfig>(() => {
    // URL params ?lab=<boardId> / ?board=<boardId> override saved state —
    // lets gallery cards deep-link. Both names accepted; `lab=` is the
    // historical one, `board=` is the obvious one.
    if (typeof window !== 'undefined') {
      const sp = new URLSearchParams(window.location.search);
      const labParam = sp.get('lab') ?? sp.get('board');
      if (labParam) {
        const fromParam = BOARD_CONFIGS.find((c) => c.boardId === labParam);
        if (fromParam) return fromParam;
      }
    }
    const savedId = localStorage.getItem('labwired-board');
    if (savedId) {
      const found = BOARD_CONFIGS.find((c) => c.boardId === savedId);
      if (found) return found;
    }
    return DEFAULT_BOARD;
  });

  // Code editor state
  const [source, setSource] = useState(() => loadBoardWorkspace(selectedBoard).source);
  const [compileErrors, setCompileErrors] = useState<CompileError[]>([]);
  const [, setCompileOutput] = useState('');
  const [compiling, setCompiling] = useState(false);
  const [showCode] = useState(false);
  const [projectsModalOpen, setProjectsModalOpen] = useState(false);
  const [embedOpen, setEmbedOpen] = useState(false);
  // Tracks the currently-loaded cloud project (null if the canvas is from
  // a board starter or unsaved). When set, "Save" overwrites this project.
  const [activeProjectId, setActiveProjectId] = useState<string | null>(null);
  const [activeProjectName, setActiveProjectName] = useState<string | null>(null);
  const [showRightSidebar, setShowRightSidebar] = useState(true);
  // Analyzer is an opt-in instrument now (was an always-on panel that froze the
  // canvas). Toggled from the SimDock Tools control; hidden by default.
  const [showAnalyzer, setShowAnalyzer] = useState(false);
  const [showIolink, setShowIolink] = useState(false);
  const embed = isEmbedMode();
  const autostartTriggeredRef = useRef(false);
  // Pre-built ELF carried by a shared link (base64 decoded). When set, Run uses
  // it directly — "if the share has a binary, we run it".
  const sharedFirmwareRef = useRef<Uint8Array | null>(null);

  // Unified UI feature flags (URL-selectable; embed flips some defaults off).
  // The frosted-glass backdrop-blur reads as a smeary haze in a small embedded
  // pane, so when `glass` is off we tag the root and a CSS rule drops every
  // backdrop-filter. Desktop keeps its glass.
  const uiFeatures = resolveUiFeatures();
  useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle('lw-no-glass', !uiFeatures.glass);
    return () => root.classList.remove('lw-no-glass');
  }, [uiFeatures.glass]);

  // Command palette mode + ref for global ⌘K shortcut
  const commandRefs = useRef<{ open: () => void; close: () => void } | null>(null);

  // Editor state
  const editor = useEditorState(
    loadBoardWorkspace(selectedBoard).diagram,
  );


  // Fetch catalog on mount
  useEffect(() => {
    trackUsage('app_loaded');
  }, []);

  // Persist selected board
  useEffect(() => {
    localStorage.setItem('labwired-board', selectedBoard.boardId);
  }, [selectedBoard]);

  // Handle board selection
  const handleBoardSelect = useCallback(
    (config: BoardConfig) => {
      const workspace = loadBoardWorkspace(config);
      setSelectedBoard(config);
      trackUsage('board_selected', { board: config.boardId });
      editor.loadDiagram(workspace.diagram);
      setSource(workspace.source);
      setCanvasValidationMessage(null);
      setInvalidPins([]);
      // Stop any running simulation
      setRunning(false);
      setBridge(null);
      setActiveSimulationConfig(null);
    },
    [editor],
  );

  // Load WASM module lazily
  const loadWasm = useCallback(async () => {
    if (wasmModule) return wasmModule;
    const wasmUrl = new URL(`${import.meta.env.BASE_URL}wasm/labwired_wasm.js`, window.location.origin);
    wasmUrl.searchParams.set('v', String(__BUILD_TIME__));
    const mod = await import(/* @vite-ignore */ wasmUrl.href);
    // Version the .wasm binary URL too. The generated init() defaults to a fixed
    // `labwired_wasm_bg.wasm` URL, which the browser/CDN cache forever — so a
    // rebuilt engine never reaches returning visitors (they keep a stale wasm
    // even though the versioned .js is fresh). Passing the busted URL fixes it.
    const wasmBinUrl = new URL(`${import.meta.env.BASE_URL}wasm/labwired_wasm_bg.wasm`, window.location.origin);
    wasmBinUrl.searchParams.set('v', String(__BUILD_TIME__));
    await mod.default({ module_or_path: wasmBinUrl.href });
    setWasmModule(mod as WasmModule);
    return mod as WasmModule;
  }, [wasmModule]);

  const launchSimulation = useCallback(async (config: ActiveSimulationConfig) => {
    let mod;
    try {
      mod = await loadWasm();
    } catch (e) {
      throw new Error(`WASM load failed: ${e instanceof Error ? e.message : String(e)}`);
    }
    let nextBridge;
    try {
      nextBridge = await SimulatorBridge.fromConfig(mod, config);
    } catch (e) {
      throw new Error(`Simulator init failed: ${e instanceof Error ? e.message : String(e)}`);
    }
    // Apply firmware-runtime quirks BEFORE we step. For Arduino-ESP32
    // boards this installs the heap-caps / timer / lock / wifi / sendHello
    // / crc8 thunks and fakes the dual-core handshake. stepBatch then
    // routes through step_with_esp32_aids to keep the handshake refreshed.
    if (config.quirks === 'esp32-arduino') {
      nextBridge.installEsp32ArduinoQuirks();
    } else if (config.quirks === 'arduino-esp32-autodiscover') {
      // Auto-discovery flavor — works for any GxEPD2 sketch (labwired-ereader)
      // whose ELF symbols are intact. Resolves thunk PCs at runtime from the
      // firmware's symbol table; also attaches the UC8151D panel model (which
      // decodes the GxEPD2_290_C90c byte stream — the SSD1680 default panel
      // doesn't understand those opcodes).
      nextBridge.installArduinoEsp32QuirksAutodiscover(config.firmware);
    }
    // If the board ships a pre-warmed boot snapshot, fetch it and apply
    // right after the quirks (which restore the thunk PCs into flash that
    // the snapshot expects). Drops heavy-firmware first-paint time from
    // ~30 s to under a second.
    if (config.bootSnapshotUrl) {
      try {
        const snapshotUrl = versionRuntimeAssetUrl(config.bootSnapshotUrl, __BUILD_TIME__);
        const resp = await fetch(snapshotUrl, { cache: 'no-store' });
        if (!resp.ok) {
          throw new Error(`HTTP ${resp.status} ${resp.statusText}`);
        }
        const blob = new Uint8Array(await resp.arrayBuffer());
        nextBridge.applyRuntimeSnapshot(blob);
        console.info(`[LabWired] applied ${blob.byteLength}B boot snapshot from ${config.bootSnapshotUrl}`);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        console.error('[LabWired] boot snapshot failed, falling back to cold boot:', e);
        setToast(`Snapshot fetch failed (${msg}) — falling back to cold boot. First paint will take ~30 s.`);
      }
    }
    setActiveSimulationConfig(config);
    setBridge(nextBridge);
    setRunning(true);
    traceRef.current = [];
    setTraceEntries([]);
  }, [loadWasm]);

  // Compile source code
  const handleCompile = useCallback(async () => {
    const diagramErrors = validateDiagram(editor.state.diagram);
    if (diagramErrors.length > 0) {
      setCanvasValidationMessage(diagramErrors[0]);
      setInvalidPins([]);
      setCompileErrors([]);
      setCompileOutput(`Wiring error: ${diagramErrors[0]}`);
      return null;
    }

    setCanvasValidationMessage(null);
    setInvalidPins([]);
    setCompiling(true);
    setCompileOutput('Compiling...\n');
    setCompileErrors([]);
    try {
      const result = await compileCode({
        source,
        language: 'arduino',
        target: selectedBoard.chipId,
      });
      setCompileErrors(result.errors);
      setCompileOutput(result.output);
      return result;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setCompileOutput(`Compile error: ${msg}`);
      return null;
    } finally {
      setCompiling(false);
    }
  }, [editor.state.diagram, source, selectedBoard.chipId]);

  // "Upload" (compile + run): convert diagram → config, init simulator
  const handleRun = useCallback(async () => {
    trackUsage('run_clicked', { board: selectedBoard.boardId });
    setLoading(true);
    setError(null);
    try {
      // The board to run is the FOREGROUND part's resolved board, so each MCU
      // in a multi-board lab (BLE sensor + collector) runs its own firmware.
      // Mirrors drawerSubject's resolution, recomputed here because drawerSubject
      // is defined later in render and can't be referenced from this callback.
      // For single-board workspaces this resolves to selectedBoard unchanged.
      const parts = editor.state.diagram.parts;
      let activePart = parts.find((p) => p.id === 'mcu');
      if (editor.state.selectedIds.size === 1) {
        const sel = parts.find((p) => editor.state.selectedIds.has(p.id));
        if (sel && mcuBoardForPart(sel, selectedBoard)) activePart = sel;
      }
      const runBoard = mcuBoardForPart(activePart, selectedBoard) ?? selectedBoard;

      // Try compiling first
      const result = await handleCompile();

      // Use compiled ELF if available, otherwise fall back to demo firmware
      let firmware: Uint8Array;
      let systemYaml: string;
      let chipYaml: string;

      const runConfig = () => resolveRunSystemConfig({
        diagram: editor.state.diagram,
        chipYaml: runBoard.chipYaml,
        bundledSystemYaml: runBoard.systemYaml,
        // Labs whose system YAML declares virtual devices the diagram emitter
        // can't reproduce (e.g. the multi-frame uds-tester) must run from the
        // bundled YAML, else the regenerated config drops/wrongs the device.
        preferDiagram: !runBoard.preferBundledSystem,
        onFallback: (msg) => {
          setCompileOutput((prev) => `${prev}\nUsing bundled system YAML — canvas not used: ${msg}`);
        },
      });

      // A shared link can carry its own pre-built binary — if it does, we run
      // it. Otherwise fall to the board's demo firmware, then to a demo matched
      // to the diagram (sourced from a curated example). Prod has no compiler,
      // so this is what makes shared labs runnable.
      const demoPath = runBoard.demoFirmwarePath ?? pickFallbackDemoFirmware(editor.state.diagram);

      if (result?.success && result.elf) {
        firmware = result.elf;
        const config = runConfig();
        systemYaml = config.systemYaml;
        chipYaml = config.chipYaml;
        setCompileOutput((prev) => prev + '\nUpload successful. Starting simulation...');
      } else if (sharedFirmwareRef.current) {
        firmware = sharedFirmwareRef.current;
        const config = runConfig();
        systemYaml = config.systemYaml;
        chipYaml = config.chipYaml;
        setCompileOutput((prev) => prev + '\nRunning firmware shipped with this link.');
      } else if (demoPath) {
        const firmwareUrl = versionRuntimeAssetUrl(demoPath, __BUILD_TIME__);
        const resp = await fetch(firmwareUrl, { cache: 'no-store' });
        if (!resp.ok) throw new Error(`Failed to load firmware: ${demoPath}`);
        firmware = new Uint8Array(await resp.arrayBuffer());
        const config = runConfig();
        systemYaml = config.systemYaml;
        chipYaml = config.chipYaml;
        setCompileOutput((prev) => prev + '\nUsing pre-built demo firmware.');
      } else {
        // No firmware anywhere. Surface a visible toast (the compile panel is
        // desktop-only) so mobile Run isn't a dead button.
        const why = result?.errors?.length
          ? `compile failed (${result.errors.length} error${result.errors.length === 1 ? '' : 's'})`
          : 'no firmware';
        const msg = `Cannot run ${runBoard.name}: ${why}. Open on a desktop to write and build code.`;
        setCompileOutput((prev) => prev + `\n${msg}`);
        setError(msg);
        setToast(msg);
        setLoading(false);
        return;
      }

      await launchSimulation({
        systemYaml,
        chipYaml,
        firmware,
        quirks: runBoard.quirks,
        bootSnapshotUrl: runBoard.bootSnapshotUrl,
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      setToast(`Run failed: ${msg}`);
      console.error('[LabWired] Run failed:', e);
    } finally {
      setLoading(false);
    }
  }, [handleCompile, launchSimulation, selectedBoard, editor.state.diagram, editor.state.selectedIds]);


  // Build the display-device list from the diagram so the loop knows what
  // to poll. Filter to types that have a known wasm framebuffer accessor.
  const displays = useMemo(
    () =>
      editor.state.diagram.parts
        .filter(
          (p) =>
            p.type === 'ssd1680_tricolor_290' ||
            p.type === 'uc8151d_tricolor_290' ||
            p.type === 'pcd8544',
        )
        .map((p) => ({
          partId: p.id,
          kind: p.type as 'ssd1680_tricolor_290' | 'uc8151d_tricolor_290' | 'pcd8544',
          // GxEPD2's red-plane inversion only applies on the SSD1680 path
          // (writeImage emits an inverted bitmap then 0x26 commits). When
          // GxEPD2 falls through to UC8151D opcodes (DTM1/DTM2) it writes
          // plane data directly with no inversion. Without this filter the
          // UC8151D's initial 0xFF / 0xFF state gets flipped to 0xFF / 0x00
          // and the whole panel renders solid red on first paint.
          invertRedPlane:
            p.type === 'ssd1680_tricolor_290' &&
            (selectedBoard.quirks === 'esp32-arduino' ||
              selectedBoard.quirks === 'arduino-esp32-autodiscover'),
        })),
    [editor.state.diagram.parts, selectedBoard.quirks],
  );

  // Phone vs desktop. Below the md breakpoint we render the touch run view
  // (MobileRunView) instead of the desktop editor; the flag also tightens the
  // sim loop's frame budget and slows display polling to save the weaker GPU.
  const [isMobile, setIsMobile] = useState(() => {
    if (typeof window === 'undefined') return false;
    return window.matchMedia('(max-width: 767px)').matches;
  });
  useEffect(() => {
    if (typeof window === 'undefined') return;
    const mq = window.matchMedia('(max-width: 767px)');
    const handler = (e: MediaQueryListEvent) => setIsMobile(e.matches);
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, []);

  // Drive the simulation loop. useSimulationLoop auto-tunes the per-frame
  // cycle batch to keep stepBatch under a ~14 ms budget — small for fast
  // firmware (Rust no_std blinky), big for heavy firmware (Arduino-ESP32
  // sketches need ~30 M cycles to reach Display::render). Seed slightly
  // higher than the hook's default so the first frame isn't tiny.
  const { state: simState, stepOnce, clearUart } = useSimulationLoop({
    bridge,
    running,
    cyclesPerFrame: 1_000_000,
    displays,
    mobile: isMobile,
  });

  // Accumulate trace entries
  const prevPcRef = useRef(0);
  if (simState.pc !== prevPcRef.current && simState.disassembly) {
    prevPcRef.current = simState.pc;
    const entry: TraceEntry = { pc: simState.pc, disassembly: simState.disassembly };
    traceRef.current = [...traceRef.current.slice(-200), entry];
    if (traceRef.current.length !== traceEntries.length) {
      setTraceEntries(traceRef.current);
    }
  }

  // Build register map
  const registers = useMemo(() => {
    if (!bridge) return new Map<string, number>();
    const names = bridge.getRegisterNames();
    const map = new Map<string, number>();
    names.forEach((name: string, i: number) => map.set(name, bridge.getRegister(i)));
    return map;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bridge, simState.pc]);

  const stackBase = registers.get('SP') ?? registers.get('R13') ?? 0x20005000;
  const stackMemory = useMemo(() => {
    if (!bridge) return new Uint8Array(0);
    return bridge.readMemory(stackBase, 64);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bridge, stackBase]);

  const handleButtonToggle = useCallback(
    (id: string, pressed: boolean) => { bridge?.setBoardIoInput(id, pressed); },
    [bridge],
  );

  const handleCompleteWire = useCallback((endpoint: { part: string; pin: string }) => {
    if (!editor.state.wireInProgress) return;
    const errorMessage = validateWireConnection(editor.state.diagram, editor.state.wireInProgress, endpoint);
    if (errorMessage) {
      editor.cancelWire();
      setCanvasValidationMessage(errorMessage);
      setInvalidPins([editor.state.wireInProgress, endpoint]);
      setCompileOutput(`Wiring error: ${errorMessage}`);
      return;
    }
    setCanvasValidationMessage(null);
    setInvalidPins([]);
    editor.completeWire(endpoint);
  }, [editor]);

  const handleStartWire = useCallback((endpoint: { part: string; pin: string }) => {
    setCanvasValidationMessage(null);
    setInvalidPins([]);
    editor.startWire(endpoint);
  }, [editor]);

  const handleCancelWire = useCallback(() => {
    setCanvasValidationMessage(null);
    setInvalidPins([]);
    editor.cancelWire();
  }, [editor]);

  const handlePlay = useCallback(() => setRunning(true), []);
  const handlePause = useCallback(() => setRunning(false), []);
  const handleStep = useCallback(() => { setRunning(false); stepOnce(); }, [stepOnce]);
  const handleReset = useCallback(async () => {
    if (!activeSimulationConfig) {
      setRunning(false);
      clearUart();
      traceRef.current = [];
      setTraceEntries([]);
      return;
    }

    setLoading(true);
    setError(null);
    try {
      setRunning(false);
      clearUart();
      await launchSimulation(activeSimulationConfig);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [activeSimulationConfig, clearUart, launchSimulation]);

  // NTC thermistor temperature state (device id -> temperature °C)
  const [ntcTemperatures, setNtcTemperatures] = useState<Record<string, number>>({});

  // SSD1306 live framebuffer
  const [ssd1306Framebuffer, setSsd1306Framebuffer] = useState<Uint8Array | null>(null);

  useEffect(() => {
    if (!running || !bridge) {
      setSsd1306Framebuffer(null);
      return;
    }
    const poll = () => {
      const fb = bridge.getSsd1306Framebuffer('oled');
      if (fb) setSsd1306Framebuffer(fb);
    };
    poll();
    const id = window.setInterval(poll, isMobile ? 250 : 100);
    return () => window.clearInterval(id);
  }, [running, bridge, isMobile]);

  // PCD8544 (Nokia 5110) live framebuffer.
  const [pcd8544Framebuffer, setPcd8544Framebuffer] = useState<Uint8Array | null>(null);
  const bridgeRef = useRef<typeof bridge>(bridge);
  useEffect(() => {
    bridgeRef.current = bridge;
  }, [bridge]);

  useEffect(() => {
    if (!running || !bridge) {
      setPcd8544Framebuffer(null);
      return;
    }
    const poll = () => {
      const fb = bridge.getPcd8544Framebuffer('lcd');
      if (fb) setPcd8544Framebuffer(fb);
    };
    poll();
    const id = window.setInterval(poll, isMobile ? 250 : 100);
    return () => window.clearInterval(id);
  }, [running, bridge, isMobile]);

  useEffect(() => {
    if (!bridge) return;
    for (const part of editor.state.diagram.parts) {
      if (part.type !== 'ultrasonic') continue;
      const cm = Number.parseFloat(part.attrs.distance ?? '');
      if (!Number.isFinite(cm)) continue;
      bridge.setHcsr04Distance(part.id, cm);
    }
  }, [bridge, editor.state.diagram.parts]);

  // NTC thermistor temperature setter (same as the desktop inspector widget,
  // extracted so the mobile inputs sheet can drive any thermistor part).
  const handleNtcChange = useCallback(
    (partId: string, tempC: number) => {
      setNtcTemperatures((prev) => ({ ...prev, [partId]: tempC }));
      bridge?.setNtcTemperature(partId, tempC);
    },
    [bridge],
  );

  // ILI9341 live framebuffer (153 KB @ 100 ms = ~1.5 MB/s WASM→JS)
  const [ili9341Framebuffer, setIli9341Framebuffer] = useState<Uint8Array | null>(null);

  useEffect(() => {
    if (!running || !bridge) {
      setIli9341Framebuffer(null);
      return;
    }
    const poll = () => {
      try {
        const fb = bridge.getIli9341Framebuffer('tft');
        if (fb) setIli9341Framebuffer(new Uint8Array(fb));
      } catch { /* device not present in this lab */ }
    };
    poll();
    const id = window.setInterval(poll, isMobile ? 250 : 100);
    return () => window.clearInterval(id);
  }, [running, bridge, isMobile]);

  const analogStates = useMemo(() => bridge?.getAnalogStates() ?? [], [bridge, simState.pc]);
  const adcDeviceStates = useMemo(() => bridge?.getAdcDeviceStates() ?? [], [bridge, simState.pc]);

  const boardIoStateMap = useMemo(() => {
    const map: Record<string, ComponentState> = {};
    const ioConfig = bridge?.getBoardIoConfig() ?? [];
    const bindingPartIds = new Map(ioConfig.map((binding) => [
      binding.id,
      resolveBindingPartId(editor.state.diagram, binding),
    ]));

    for (const s of simState.boardIoStates) {
      const partId = bindingPartIds.get(s.id) ?? s.id;
      map[partId] = { ...(map[partId] ?? {}), active: s.active };
    }

    for (const a of analogStates) {
      const partId = bindingPartIds.get(a.id) ?? a.id;
      if (!map[partId]) map[partId] = {};
      if (a.kind === 'adc_input' && a.value !== undefined) {
        map[partId].analogValue = a.value;
      }
      if (a.kind === 'pwm_output') {
        map[partId].active = a.active;
      }
    }

    if (bridge) {
      for (const binding of ioConfig) {
        const partId = bindingPartIds.get(binding.id) ?? binding.id;
        if (binding.kind !== 'pwm_output' || !map[partId]) continue;
        try {
          const snap = bridge.getPeripheralSnapshot(binding.peripheral) as
            { psc?: number; arr?: number; cnt?: number } | null;
          if (snap && typeof snap.arr === 'number' && snap.arr > 0) {
            const clockHz = 8_000_000;
            const freq = Math.round(clockHz / ((snap.psc ?? 0) + 1) / (snap.arr + 1));
            map[partId].frequency = freq;
            if (freq >= 40 && freq <= 60) {
              map[partId].angle = map[partId].active ? 90 : 0;
            }
          }
        } catch {
          // Peripheral might not support snapshot
        }
      }
    }

    return map;
  }, [simState.boardIoStates, analogStates, bridge, editor.state.diagram]);

  // Interactive analog component handler
  const handleAnalogChange = useCallback(
    (partId: string, value: number) => {
      if (!bridge) return;
      const config = bridge.getBoardIoConfig();
      const binding = config.find((b) => b.id === partId);
      if (binding) {
        bridge.setAdcValue(binding.peripheral, value);
      }
    },
    [bridge],
  );


  const handleDropPart = useCallback(
    (type: string, x: number, y: number) => {
      const def = COMPONENT_REGISTRY.get(type);
      if (!def) return;
      const part: Part = {
        id: nextPartId(type), type, x, y, rotate: 0,
        attrs: { ...def.defaultAttrs },
      };
      editor.addPart(part);
    },
    [editor],
  );

  const isEmpty = editor.state.diagram.parts.filter((p) => p.id !== 'mcu').length === 0;

  // Inspector: derive selection from selectedIds (parts only; wires have no stable id in this schema)
  const inspectorSelection = useMemo<InspectorSelection | null>(() => {
    if (editor.state.selectedIds.size !== 1) return null;
    const selectedId = [...editor.state.selectedIds][0];
    const part = editor.state.diagram.parts.find((p) => p.id === selectedId);
    if (!part) return null;
    const def = COMPONENT_REGISTRY.get(part.type);
    return {
      kind: 'part',
      partId: part.id,
      partType: part.type,
      label: def?.label ?? part.type,
      pins: (def?.pins ?? []).map((p: { id: string; label?: string }) => ({ id: p.id, label: p.label ?? p.id })),
      attrs: part.attrs ?? {},
    };
  }, [editor.state.selectedIds, editor.state.diagram.parts]);

  // The dev drawer reflects the MCU selected on the canvas, falling
  // back to the primary 'mcu'. This replaces the old activeChipId
  // coupling: dropping a 2nd MCU (e.g. an nRF52840 DK) and clicking
  // it now re-binds the drawer to that chip. Only the primary 'mcu'
  // has a live simulator bridge; secondary MCUs show their static
  // identity (Source / YAML) with no live serial/registers.
  const drawerSubject = useMemo(() => {
    const parts = editor.state.diagram.parts;
    const primaryPart = parts.find((p) => p.id === 'mcu');
    let selectedMcu: typeof primaryPart | undefined;
    if (editor.state.selectedIds.size === 1) {
      const id = [...editor.state.selectedIds][0];
      const p = parts.find((pp) => pp.id === id);
      if (p && mcuBoardForPart(p, selectedBoard)) selectedMcu = p;
    }
    const part = selectedMcu ?? primaryPart;
    const board = mcuBoardForPart(part, selectedBoard) ?? selectedBoard;
    return { part, board, isPrimary: part?.id === 'mcu' };
  }, [editor.state.selectedIds, editor.state.diagram.parts, selectedBoard]);

  // Per-chip sims: the selected MCU is the foreground (App's bridge/running
  // mirror it, the main loop drives it); every other running chip ticks in
  // the background so two chips can talk over the shared BLE air at once.
  const foregroundPartId = drawerSubject.part?.id ?? 'mcu';
  const mcuPartIds = useMemo(
    () => editor.state.diagram.parts.filter((p) => mcuBoardForPart(p, selectedBoard)).map((p) => p.id),
    [editor.state.diagram.parts, selectedBoard],
  );
  const { sims: chipSims } = usePerChipSims({
    foregroundPartId,
    mcuPartIds,
    bridge,
    running,
    config: activeSimulationConfig,
    board: drawerSubject.board,
    foregroundUart: simState.uartOutput,
    setBridge,
    setRunning,
    setConfig: setActiveSimulationConfig,
    clearUart,
  });

  const clearChipSerial = (partId: string) => {
    if (partId === foregroundPartId) clearUart();
    const sim = chipSims.current.get(partId);
    if (sim) sim.uart = '';
  };
  const chipSerialFor = (partId: string, isFg: boolean) => {
    const sim = chipSims.current.get(partId);
    return isFg ? (sim?.uart ?? '') + simState.uartOutput : sim?.uart ?? '';
  };

  // Floating property windows — one per clicked component (any part, not just
  // chips). Click a part → its window opens NEAR the click; × closes it;
  // removing the part prunes it (the render list filters to live parts).
  const lastPointerRef = useRef({ x: 220, y: 150 });
  useEffect(() => {
    const onDown = (e: PointerEvent) => {
      lastPointerRef.current = { x: e.clientX, y: e.clientY };
    };
    window.addEventListener('pointerdown', onDown, true);
    return () => window.removeEventListener('pointerdown', onDown, true);
  }, []);
  const [openWindows, setOpenWindows] = useState<{ id: string; x: number; y: number }[]>([]);
  const openWindowIds = openWindows.filter((w) =>
    editor.state.diagram.parts.some((p) => p.id === w.id),
  );
  const openWindow = (id: string) =>
    setOpenWindows((prev) => {
      if (prev.some((w) => w.id === id)) return prev;
      const { x, y } = lastPointerRef.current;
      const px = Math.max(8, Math.min(window.innerWidth - 360, x + 16));
      const py = Math.max(8, Math.min(window.innerHeight - 200, y + 12));
      return [...prev, { id, x: px, y: py }];
    });
  const closeWindow = (id: string) =>
    setOpenWindows((prev) => prev.filter((w) => w.id !== id));
  const openLogicAnalyzerTool = () => {
    const existing = editor.state.diagram.parts.find((part) => part.type === 'logic-analyzer');
    if (existing) {
      openWindow(existing.id);
      return;
    }
    const def = COMPONENT_REGISTRY.get('logic-analyzer');
    if (!def) return;
    const id = nextPartId('logic-analyzer');
    editor.addPart({
      id,
      type: 'logic-analyzer',
      x: 620,
      y: 180,
      rotate: 0,
      attrs: { ...def.defaultAttrs },
    });
    openWindow(id);
  };

  // Auto-open instruments a board/lab declares (openInstruments), so a shared
  // link shows its output immediately. 'logic-analyzer' opens the UDS/CAN logic
  // analyzer window for the board's probe — NOT the BLE air tracer.
  useEffect(() => {
    if (selectedBoard.openInstruments?.includes('logic-analyzer')) {
      openLogicAnalyzerTool();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedBoard.boardId]);

  const renderRuntimeControl = (part: Part) =>
    renderComponentRuntimeControl({
      part,
      bridge,
      updateAttrs: (partId, attrs) => editor.updateAttrs(partId, attrs),
    });

  // Build live sensor widget for selected I2C / UART devices
  const inspectorLabWidget = useMemo<ReactNode>(() => {
    if (!inspectorSelection || inspectorSelection.kind !== 'part') return undefined;
    const partType = inspectorSelection.partType;
    if (partType === 'oled-ssd1306') {
      if (!bridge) return undefined;
      return <Ssd1306Display framebuffer={ssd1306Framebuffer} width={256} />;
    }
    if (partType === 'pcd8544' || partType === 'nokia-5110') {
      return <Pcd8544Display framebuffer={pcd8544Framebuffer} width={252} />;
    }
    if (partType === 'ili9341') {
      if (!bridge) return undefined;
      return <Ili9341Display framebuffer={ili9341Framebuffer} width={240} />;
    }
    if (partType === 'neo6m-gps') {
      if (!bridge) return undefined;
      const gpsStates = bridge.getUartDeviceStates();
      const s = gpsStates.find((st) => st.kind === 'neo6m-gps' && st.id === inspectorSelection.partId)
        ?? gpsStates.find((st) => st.kind === 'neo6m-gps');
      if (!s || s.kind !== 'neo6m-gps') return undefined;
      return (
        <GpsControl
          lat={s.lat}
          lon={s.lon}
          hasFix={s.has_fix}
          onChange={(lat, lon) => bridge.setGpsPosition(inspectorSelection.partId, lat, lon)}
          onFixToggle={(active) => bridge.setGpsFix(inspectorSelection.partId, active)}
        />
      );
    }
    if (partType === 'ntc-thermistor') {
      if (!bridge) return undefined;
      const partId = inspectorSelection.partId;
      const s = adcDeviceStates.find((st) => st.kind === 'ntc-thermistor' && st.id === partId)
        ?? adcDeviceStates.find((st) => st.kind === 'ntc-thermistor');
      const tempC = ntcTemperatures[partId] ?? 25.0;
      return (
        <ThermistorControl
          temperatureC={tempC}
          dividerMv={s?.divider_mv}
          adcCount={s?.adc_count}
          onChange={(t) => {
            setNtcTemperatures((prev) => ({ ...prev, [partId]: t }));
            bridge.setNtcTemperature(partId, t);
          }}
        />
      );
    }
    if (partType === 'sn74hc165') {
      const part = editor.state.diagram.parts.find((candidate) => candidate.id === inspectorSelection.partId);
      return part ? renderRuntimeControl(part) : undefined;
    }
    if (partType === 'iolink-master') {
      if (!bridge) return undefined;
      // Read-only readout of the IO-Link master peer's live process data.
      const s = bridge.getIolinkMasterState();
      return (
        <table className="w-full text-[12px] font-mono">
          <tbody>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">Link</td><td className="text-fg-primary">{s?.link_state ?? 'offline'}</td></tr>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">PD valid</td><td className="text-fg-primary">{s?.pd_valid ? 'yes' : 'no'}</td></tr>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">Process data</td><td className="text-fg-primary">0x{(s?.input_byte ?? 0).toString(16).toUpperCase().padStart(2, '0')}</td></tr>
          </tbody>
        </table>
      );
    }
    if (partType !== 'adxl345' && partType !== 'mpu6050' && partType !== 'bme280') return undefined;
    if (!bridge) return undefined;
    const sensorStates = bridge.getI2cSensorStates();
    if (partType === 'adxl345') {
      const s = sensorStates.find((st) => st.kind === 'adxl345' && st.id === inspectorSelection.partId)
        ?? sensorStates.find((st) => st.kind === 'adxl345');
      if (!s || s.kind !== 'adxl345') return undefined;
      return (
        <table className="w-full text-[12px] font-mono">
          <tbody>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">X</td><td className="text-fg-primary">{s.x}</td></tr>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">Y</td><td className="text-fg-primary">{s.y}</td></tr>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">Z</td><td className="text-fg-primary">{s.z}</td></tr>
          </tbody>
        </table>
      );
    }
    if (partType === 'mpu6050') {
      const s = sensorStates.find((st) => st.kind === 'mpu6050' && st.id === inspectorSelection.partId)
        ?? sensorStates.find((st) => st.kind === 'mpu6050');
      if (!s || s.kind !== 'mpu6050') return undefined;
      return (
        <table className="w-full text-[12px] font-mono">
          <tbody>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">AX</td><td className="text-fg-primary">{s.ax}</td></tr>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">AY</td><td className="text-fg-primary">{s.ay}</td></tr>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">AZ</td><td className="text-fg-primary">{s.az}</td></tr>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">GX</td><td className="text-fg-primary">{s.gx}</td></tr>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">GY</td><td className="text-fg-primary">{s.gy}</td></tr>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">GZ</td><td className="text-fg-primary">{s.gz}</td></tr>
          </tbody>
        </table>
      );
    }
    if (partType === 'bme280') {
      const s = sensorStates.find((st) => st.kind === 'bme280' && st.id === inspectorSelection.partId)
        ?? sensorStates.find((st) => st.kind === 'bme280');
      if (!s || s.kind !== 'bme280') return undefined;
      return (
        <table className="w-full text-[12px] font-mono">
          <tbody>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">Temp</td><td className="text-fg-primary">{s.temperature_c.toFixed(1)} °C</td></tr>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">Humidity</td><td className="text-fg-primary">{s.humidity_pct.toFixed(1)} %RH</td></tr>
            <tr><td className="py-0.5 pr-2 text-fg-secondary">Pressure</td><td className="text-fg-primary">{s.pressure_hpa.toFixed(0)} hPa</td></tr>
          </tbody>
        </table>
      );
    }
    return undefined;
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bridge, editor.state.diagram.parts, inspectorSelection, simState.pc, ssd1306Framebuffer, pcd8544Framebuffer, ili9341Framebuffer, adcDeviceStates, ntcTemperatures]);

  // Right-side InspectorCard removed — Properties live in the
  // bottom drawer (per-chip Serial/Registers/Trace/Memory/Source/
  // YAML). The part-specific inspector (lab widgets, delete/
  // duplicate) is dropped along with it; can be reintroduced as a
  // tab on the properties drawer if useful later.
  void inspectorSelection;

  const paletteComponents = useMemo<PaletteComponent[]>(
    () =>
      Array.from(COMPONENT_REGISTRY.entries())
        .filter(([type]) => type !== 'mcu' && !type.startsWith('boards/'))
        .map(([type, def]) => {
          const category = PALETTE_CATEGORY[type] ?? 'misc';
          return {
            type,
            label: def.label ?? type,
            category,
            bus: undefined,
            thumb: getComponentIcon(type, category),
          };
        }),
    []
  );

  const handlePaletteDrag = (componentType: string) => {
    // The dataTransfer is set inside PaletteDrawer; this callback is purely informational
    // for any future analytics or drag-state tracking. No-op for now.
    void componentType;
  };

  const simDockState: StudioSimState = useMemo(() => {
    if (loading) return 'building';
    if (running) return 'running';
    if (bridge && !running) return 'paused';
    return 'idle';
  }, [loading, running, bridge]);

  const handlePickLab = useCallback(
    (labId: string) => {
      const next = BOARD_CONFIGS.find((b) => b.boardId === labId);
      if (!next) return;
      trackUsage('lab_opened', { board: next.boardId });
      const workspace = loadBoardWorkspace(next);
      setSelectedBoard(next);
      editor.loadDiagram(workspace.diagram);
      setSource(workspace.source);
      setCanvasValidationMessage(null);
      setInvalidPins([]);
      setRunning(false);
      setBridge(null);
      setActiveSimulationConfig(null);
    },
    [editor],
  );

  const handleUploadFirmware = useCallback(
    async (file: File) => {
      try {
        setError(null);
        setCompileOutput(`Loading firmware: ${file.name} (${(file.size / 1024).toFixed(1)} KB)`);
        const firmware = new Uint8Array(await file.arrayBuffer());

        // Upload targets the SELECTED chip. The primary 'mcu' owns the wired
        // canvas, so derive its system YAML from the diagram. A secondary chip
        // boots standalone against its own board YAML — cross-chip comms ride
        // the shared BLE air, not wires.
        const target = drawerSubject.board;
        const config = resolveRunSystemConfig({
          diagram: editor.state.diagram,
          chipYaml: target.chipYaml,
          bundledSystemYaml: target.systemYaml,
          preferDiagram: drawerSubject.isPrimary,
          onFallback: (msg) => {
            setCompileOutput((prev) => `${prev}\nUsing bundled system YAML — canvas not used: ${msg}`);
          },
        });

        await launchSimulation({
          systemYaml: config.systemYaml,
          chipYaml: config.chipYaml,
          firmware,
          quirks: target.quirks,
          bootSnapshotUrl: target.bootSnapshotUrl,
        });
        setCompileOutput((prev) => `${prev}\nUploaded ${file.name} to ${target.name} (${firmware.length} bytes). Simulation started.`);
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        setError(`Upload failed: ${message}`);
        setCompileOutput((prev) => `${prev}\nUpload failed: ${message}`);
      }
    },
    [launchSimulation, drawerSubject, editor.state.diagram],
  );

  const handlePartAttrChange = useCallback(
    (partId: string, attrs: Record<string, string>) => {
      editor.updateAttrs(partId, attrs);
      const part = editor.state.diagram.parts.find((p) => p.id === partId);
      if (!part) return;
      for (const [key, value] of Object.entries(attrs)) {
        syncSensorAttributeToSimulator({
          partId,
          partType: part.type,
          key,
          value,
          bridge: bridgeRef.current,
        });
      }
    },
    [editor],
  );

  // Load from short share URL, hash sharing, or localStorage
  useEffect(() => {
    const shareId = new URLSearchParams(window.location.search).get('share');
    if (shareId) {
      fetchSharedProject(shareId).then((project) => {
        if (project) {
          const prepared = prepareSharedProjectForPlayground(project.diagram);
          if (!prepared) {
            setCanvasValidationMessage(`Unsupported shared board: ${project.diagram.board}`);
            setActiveProjectId(null);
            setActiveProjectName(`Unsupported board: ${project.diagram.board}`);
            return;
          }
          setSelectedBoard(prepared.board);
          editor.loadDiagram(prepared.diagram);
          setSource(project.source);
          // If the share shipped a pre-built binary, run that exact firmware.
          sharedFirmwareRef.current = project.firmware
            ? Uint8Array.from(atob(project.firmware), (c) => c.charCodeAt(0))
            : null;
          setActiveProjectId(null);
          setActiveProjectName(null);
          setCanvasValidationMessage(null);
          setInvalidPins([]);
          setRunning(false);
          setBridge(null);
          setActiveSimulationConfig(null);
          for (const p of prepared.diagram.parts) {
            const num = parseInt(p.id.replace(/\D/g, ''), 10);
            if (!isNaN(num) && num > partCounter) partCounter = num;
          }
        }
      });
      return;
    }

    const hash = window.location.hash.slice(1);
    if (hash) {
      decodeProject(hash).then((project) => {
        if (project) {
          const prepared = prepareSharedProjectForPlayground(project.diagram);
          if (!prepared) {
            setCanvasValidationMessage(`Unsupported shared board: ${project.diagram.board}`);
            setActiveProjectId(null);
            setActiveProjectName(`Unsupported board: ${project.diagram.board}`);
            return;
          }
          setSelectedBoard(prepared.board);
          editor.loadDiagram(prepared.diagram);
          setSource(project.source);
          setActiveProjectId(null);
          setActiveProjectName(null);
          setCanvasValidationMessage(null);
          setInvalidPins([]);
          setRunning(false);
          setBridge(null);
          setActiveSimulationConfig(null);
          for (const p of prepared.diagram.parts) {
            const num = parseInt(p.id.replace(/\D/g, ''), 10);
            if (!isNaN(num) && num > partCounter) partCounter = num;
          }
          history.replaceState(null, '', window.location.pathname + window.location.search);
          return;
        }
      });
      return;
    }

    const workspace = loadBoardWorkspace(selectedBoard);
    editor.loadDiagram(workspace.diagram);
    setSource(workspace.source);
    for (const p of workspace.diagram.parts) {
      const num = parseInt(p.id.replace(/\D/g, ''), 10);
      if (!isNaN(num) && num > partCounter) partCounter = num;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (autostartTriggeredRef.current || embed) return;
    const hash = window.location.hash.slice(1);
    if (hash) return;
    if (selectedBoard.boardId !== DEFAULT_BOARD.boardId) return;
    if (hasSavedWorkspace(selectedBoard.boardId)) return;
    if (localStorage.getItem(DEMO_AUTOSTART_KEY)) return;

    autostartTriggeredRef.current = true;
    localStorage.setItem(DEMO_AUTOSTART_KEY, '1');
    void handleRun();
  }, [embed, handleRun, selectedBoard.boardId]);

  // ?run=1 — auto-click Run once the board is loaded. Used by the watch overlay
  // iframe ("agent picked this board, show me the sim running"). Unconditional:
  // overrides the default-board guard above and the autostart localStorage key.
  useEffect(() => {
    if (autostartTriggeredRef.current) return;
    const wantsAutoRun = new URLSearchParams(window.location.search).get('run') === '1';
    if (!wantsAutoRun) return;
    autostartTriggeredRef.current = true;
    void handleRun();
  }, [handleRun]);

  // "If we have a binary, run it by default, so we can see things." A deep-linked
  // example/lab (?lab= / ?board=) that ships a pre-built binary (its own
  // demoFirmwarePath, or one matched to the diagram) autostarts on open.
  useEffect(() => {
    if (autostartTriggeredRef.current || embed) return;
    const sp = new URLSearchParams(window.location.search);
    if (!(sp.get('lab') ?? sp.get('board'))) return;
    const hasBinary = !!selectedBoard.demoFirmwarePath || !!pickFallbackDemoFirmware(editor.state.diagram);
    if (!hasBinary) return;
    autostartTriggeredRef.current = true;
    void handleRun();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [embed, handleRun, selectedBoard.boardId, selectedBoard.demoFirmwarePath]);

  useEffect(() => {
    if (selectedBoard.kind === 'lab') return;
    localStorage.setItem(
      getWorkspaceStorageKey(selectedBoard.boardId, 'diagram'),
      JSON.stringify(editor.state.diagram),
    );
  }, [editor.state.diagram, selectedBoard]);

  useEffect(() => {
    if (selectedBoard.kind === 'lab') return;
    localStorage.setItem(getWorkspaceStorageKey(selectedBoard.boardId, 'source'), source);
  }, [source, selectedBoard]);




  // Studio-shell toast (transient, auto-dismisses)
  const [toast, setToast] = useState<string | null>(null);

  // Clerk handles sign-in directly via <SignInButton mode="modal"> in AuthPill.
  // The cabinet (AccountPanel) shows the API key + Clerk profile, opened from
  // anywhere via setAccountOpen — currently triggered by UserButton's profile
  // hook in a follow-up; for now it's reachable from URL fragment.
  const [accountOpen, setAccountOpen] = useState(false);

  // Sign-in gate: anonymous browse is fine, but Run / Step (anything that
  // consumes simulator cycles) requires a Clerk account on production. This
  // is the primary conversion lever — users come in, browse, hit Run, sign
  // in, become users.
  //
  // Fail-open when Clerk hasn't loaded: in local dev (and the rare prod
  // outage) the production publishable key rejects non-labwired.com
  // origins with HTTP 400, so `isLoaded` stays false forever. Previously
  // `if (!clerkLoaded) return;` silently swallowed every Run click —
  // looked like "click does nothing." Treat unloaded Clerk as
  // anonymous-but-allowed so the simulator is usable; the production
  // domain still gets the real gate because Clerk loads successfully there.
  const { isSignedIn, isLoaded: clerkLoaded } = useUser();
  const { getToken } = useAuth();
  const { openSignIn } = useClerk();

  // Build the auth + preview-image extras for a share/embed POST. ONLY
  // signed-in users get a per-lab card: we render the board <svg> to a PNG and
  // attach a Clerk token (the API stores the image only for authed requests).
  // Anonymous users (and ANY failure) get an empty options object → the share
  // still works, card falls back to the LabWired logo. Never throws.
  const buildShareExtras = useCallback(async (): Promise<ShareOptions> => {
    if (!isSignedIn) return {};
    try {
      const svg = document.querySelector('svg.editor-canvas') as SVGSVGElement | null;
      const [previewPng, authToken] = await Promise.all([
        svg ? renderCanvasPng(svg) : Promise.resolve(null),
        getToken(),
      ]);
      const extras: ShareOptions = {};
      if (previewPng) extras.previewPng = previewPng;
      if (authToken) extras.authToken = authToken;
      return extras;
    } catch {
      return {};
    }
  }, [isSignedIn, getToken]);
  const requireAuth = useCallback(
    (action: () => void) => {
      // Local-dev escape hatch: set VITE_DISABLE_AUTH=true in
      // packages/playground/.env.local to run sims without signing in.
      // Off by default, so production (Cloudflare Pages) stays gated.
      if (import.meta.env.VITE_DISABLE_AUTH === 'true') {
        action();
        return;
      }
      // Embedded in a host (e.g. the ChatGPT app): the only way to reach this
      // iframe is through an MCP tool call that already authenticated via the
      // connector's OAuth. A second Clerk sign-in inside the embed is redundant
      // and impossible to complete in that sandbox, so fail open. The sim runs
      // in-browser (WASM), so there is no server cost behind this gate.
      if (embed) {
        action();
        return;
      }
      if (clerkLoaded && !isSignedIn) {
        openSignIn({});
        return;
      }
      action();
    },
    [embed, clerkLoaded, isSignedIn, openSignIn],
  );

  // Wall-clock runtime tracker — ticks while the simulation is running.
  // Frozen on pause, reset to 0 when the simulation is reset.
  const [runtimeMs, setRuntimeMs] = useState(0);
  const runStartRef = useRef<number | null>(null);

  useEffect(() => {
    if (running) {
      runStartRef.current = Date.now() - runtimeMs;
      const tick = () => {
        if (runStartRef.current !== null) {
          setRuntimeMs(Date.now() - runStartRef.current);
        }
      };
      tick();
      const interval = window.setInterval(tick, 500);
      return () => window.clearInterval(interval);
    }
    runStartRef.current = null;
    return undefined;
    // We intentionally exclude `runtimeMs` from deps — including it would re-create
    // the interval on every tick. The ref captures the latest value on `running` transitions.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [running]);

  useEffect(() => {
    // Reset elapsed time whenever the active simulation is cleared (reset / board switch).
    if (activeSimulationConfig === null) {
      setRuntimeMs(0);
      runStartRef.current = null;
    }
  }, [activeSimulationConfig]);

  // Share
  const handleShare = useCallback(async () => {
    try {
      const extras = await buildShareExtras();
      const url = await generateShareUrl(editor.state.diagram, source, extras);
      await navigator.clipboard.writeText(url);
      // Warn (don't block) when the shared link can't actually run, so we stop
      // minting dead shares that open to a circuit nobody can Run.
      setToast(
        sharedCircuitIsRunnable(selectedBoard, source)
          ? 'Share URL copied to clipboard'
          : "Link copied — but this circuit has no code to run yet, so it won't run when opened. Write and run code first, or share a built-in example.",
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setToast(`Share failed: ${message}`);
    }
  }, [editor.state.diagram, source, selectedBoard, buildShareExtras]);

  // Embed — opens the dialog that mints embed code + a live preview.
  const handleEmbed = useCallback(() => {
    setEmbedOpen(true);
  }, []);

  // Keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA') return;
      if ((e.target as HTMLElement).closest('.monaco-editor')) return;

      if (e.key === 'Delete' || e.key === 'Backspace') {
        if (editor.state.selectedIds.size > 0) {
          editor.deleteSelected();
        }
      }
      if (e.key === 'r' || e.key === 'R') {
        if (editor.state.selectedIds.size === 1) {
          editor.rotatePart([...editor.state.selectedIds][0]);
        }
      }
      if (e.ctrlKey && e.shiftKey && e.key === 'Z') {
        e.preventDefault(); editor.redo();
      } else if (e.ctrlKey && e.key === 'z') {
        e.preventDefault(); editor.undo();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [editor]);

  // Global ⌘K shortcut — opens command palette
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        commandRefs.current?.open();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, []);


  // Bridge so the command palette (defined here, outside the
  // ChipsProvider tree) can drop a new MCU through the ⌘K flow.
  const addMcuRef = useRef<((board?: BoardConfig) => void) | null>(null);
  // Bridge to open the properties drawer when the user clicks the
  // MCU part on the EditorCanvas — chip-on-canvas IS the affordance.
  const setPropsOpenRef = useRef<((open: boolean) => void) | null>(null);

  // Command palette items
  const commandItems = useCommandPaletteItems({
    boards: pickerBoards(),
    onLoadBoard: handleBoardSelect,
    onPickLab: handlePickLab,
    onAddComponent: (type) => {
      const def = COMPONENT_REGISTRY.get(type);
      if (!def) return;
      const part: Part = {
        id: nextPartId(type), type, x: 200, y: 200, rotate: 0,
        attrs: { ...def.defaultAttrs },
      };
      editor.addPart(part);
    },
    onRun: () => requireAuth(handleRun),
    onShare: handleShare,
    onReset: handleReset,
    onAddMcu: (board) => addMcuRef.current?.(board),
  });

  const renderCommandPalette = (open: boolean, closeCommand: () => void, _openCommand: () => void) => (
    <CommandPalette open={open} onClose={closeCommand} items={commandItems} />
  );

  // Run-button intent: if a sim is already loaded, resume from pause; otherwise launch fresh.
  const onSimRun = activeSimulationConfig ? handlePlay : handleRun;

  // The per-chip control surface (Run/Pause · Upload · Restart), reused by the
  // on-canvas toolbar and the inspector header. The controls always target the
  // SELECTED chip (= the foreground sim). Upload is available for any MCU
  // (booting an ELF is how a chip starts); Run/Restart light up once the chip
  // has a runnable config — for the primary that's its source, for any other
  // chip that's an uploaded firmware.
  const renderChipControls = (isPrimary: boolean, variant: 'toolbar' | 'header') => {
    const canRun = isPrimary || !!activeSimulationConfig || !!drawerSubject.board?.demoFirmwarePath;
    return (
      <ChipControls
        variant={variant}
        state={simDockState}
        canRun={canRun}
        canUpload
        onRun={() => requireAuth(onSimRun)}
        onPause={handlePause}
        onRestart={handleReset}
        onUpload={handleUploadFirmware}
        disabledReason={canRun ? undefined : 'Upload firmware to run this chip'}
      />
    );
  };

  // Cycle-consuming actions are gated behind Clerk sign-in. Anonymous users
  // who click Run get the Clerk modal instead. Pause/Reset stay open — they
  // don't consume cycles and tend to be reached only mid-flow anyway.
  const showRunHint = simDockState === 'idle' && (simState.cycles ?? 0) === 0;
  const simDockNode = (
    <div className="flex flex-col items-center gap-2">
      {showRunHint && (
        <div
          // Light, single contextual hint — not a heavy glowing box. Board
          // runHints are authored for desktop; map their "(toolbar)" reference
          // to the mobile drawer's analyzer tab.
          className="max-w-[92vw] px-3 py-1.5 rounded-2xl bg-white/[0.04] border border-white/[0.08] text-fg-secondary text-[11.5px] flex items-start gap-1.5 text-center leading-snug"
        >
          <span aria-hidden className="mt-px text-accent">▶</span>
          <span className="break-words">
            {(selectedBoard.runHint ?? 'Tap Run to start the simulation').replace(/\(toolbar\)/gi, '(BLE tab)')}
          </span>
        </div>
      )}
      <SimDock
        state={simDockState}
        runtimeMs={runtimeMs}
        cycles={simState.cycles}
        pc={simState.pc}
        onRun={() => requireAuth(onSimRun)}
        onPause={handlePause}
        onStep={() => requireAuth(handleStep)}
        onReset={handleReset}
      />
    </div>
  );

  const renderDevDrawer = (devMode: boolean, leftOffset: number) => {
    // The drawer reflects the FOREGROUND chip (= the selected MCU). Its live
    // serial/registers/trace/memory exist whenever that chip has a running
    // bridge — App's `bridge`/`simState` mirror the foreground sim. A chip
    // with no firmware yet shows a prompt to run/upload.
    const { board: subjectBoard, isPrimary } = drawerSubject;
    const hasLiveSim = !!bridge;
    const noSimLabel = `${subjectBoard.name} — run or upload firmware to see live data.`;
    // On desktop, every component is inspected in its own floating window
    // (rendered below) — the docked drawer is mobile-only now.
    if (!isMobile) return null;
    return (
    <PropertiesGate>
    <DevDrawer
      header={
        <div className="flex items-center gap-3">
          <ChipTabsBar name={subjectBoard.name} />
          {renderChipControls(isPrimary, 'header')}
        </div>
      }
      headerRight={<DrawerCloseButton />}
      devMode={devMode}
      leftOffset={leftOffset}
      tabs={{
        serial: (
          hasLiveSim ? (
            <SerialMonitor
              output={chipSerialFor(foregroundPartId, true)}
              onClear={() => clearChipSerial(foregroundPartId)}
              style={{ height: '100%' }}
            />
          ) : (
            <EmptyTabState label={noSimLabel} />
          )
        ),
        registers: (
          hasLiveSim ? (
            <RegisterGrid registers={registers} style={{ maxHeight: '100%', overflow: 'auto' }} />
          ) : (
            <EmptyTabState label="Run or upload firmware to inspect CPU registers." />
          )
        ),
        trace: (
          hasLiveSim ? (
            <InstructionTrace entries={traceEntries} style={{ maxHeight: '100%', overflow: 'auto' }} />
          ) : (
            <EmptyTabState label="Run or upload firmware to see the instruction trace." />
          )
        ),
        memory: (
          hasLiveSim ? (
            <MemoryInspector data={stackMemory} baseAddress={stackBase} style={{ maxHeight: '100%', overflow: 'auto' }} />
          ) : (
            <EmptyTabState label="Run or upload firmware to inspect memory." />
          )
        ),
        source: (
          subjectBoard.sourceCode ? (
            <div className="h-full flex flex-col">
              {subjectBoard.sourceFilename && (
                <div className="px-3 py-1.5 text-fg-tertiary text-[11px] font-mono border-b border-border bg-bg-elevated/40 shrink-0">
                  {subjectBoard.sourceFilename}
                </div>
              )}
              <pre className="font-mono text-[12px] leading-[1.5] p-3 text-fg-secondary whitespace-pre overflow-auto flex-1">
                {subjectBoard.sourceCode}
              </pre>
            </div>
          ) : (
            <div className="p-4 text-fg-tertiary text-sm">Source not bundled for this lab.</div>
          )
        ),
        yaml: (
          <pre className="font-mono text-[12px] p-3 text-fg-secondary whitespace-pre-wrap">
            {subjectBoard.systemYaml}
          </pre>
        ),
      }}
    />
    </PropertiesGate>
    );
  };

  // Saved-projects modal — rendered in both the desktop and mobile trees so the
  // phone menu's "My projects" works too. Defined once to avoid duplicating the
  // load/save wiring.
  const projectsModalNode = (
    <ProjectsModal
      open={projectsModalOpen}
      onClose={() => setProjectsModalOpen(false)}
      currentBoardId={selectedBoard.boardId}
      currentDiagramJson={JSON.stringify(editor.state.diagram)}
      currentSourceCode={source}
      activeProjectId={activeProjectId}
      activeProjectName={activeProjectName}
      onCreated={(p: ProjectRecord) => {
        setActiveProjectId(p.id);
        setActiveProjectName(p.name);
      }}
      onSaved={(p: ProjectRecord) => {
        setActiveProjectId(p.id);
        setActiveProjectName(p.name);
      }}
      onLoad={(p: ProjectRecord) => {
        // Find the matching board config — projects are tied to a board for
        // chip/system context, so we have to swap board too if the loaded
        // project is for a different one.
        const cfg = BOARD_CONFIGS.find((b: BoardConfig) => b.boardId === p.board_id);
        if (cfg && cfg.boardId !== selectedBoard.boardId) {
          handleBoardSelect(cfg);
        }
        try {
          const parsed = JSON.parse(p.diagram_json);
          editor.loadDiagram(parsed);
        } catch {
          /* malformed diagram in stored project — keep current canvas */
        }
        if (p.source_code !== null) setSource(p.source_code);
        setActiveProjectId(p.id);
        setActiveProjectName(p.name);
      }}
    />
  );

  // The embed (`?embed=true`) is a read-only demo runner: see properties, play
  // with the model (run, press buttons, use the BLE/logic/IO-Link tools), but no
  // authoring. That is exactly the mobile run shell — reuse it for embeds too,
  // on any viewport, instead of stripping the desktop editor. `resolveUiFeatures`
  // already turns the menu off in embed, so its nav drawer never shows.
  if (isMobile || embed) {
    // MCU parts in the diagram → the multi-chip switcher. Foreground is the
    // selected chip (foregroundPartId); tapping selects another so App mirrors
    // that chip's bridge/sim/serial.
    const mcuChips = editor.state.diagram.parts
      .filter((p) => mcuBoardForPart(p, selectedBoard))
      .map((p) => ({ id: p.id, name: mcuBoardForPart(p, selectedBoard)?.name ?? p.id }));
    return (
      <ChipsProvider initialBoard={selectedBoard}>
        <AddMcuRefSync addMcuRef={addMcuRef} />
    <SetPropsOpenRefSync setPropsOpenRef={setPropsOpenRef} />
        <ChipBridgeSync
          bridge={bridge}
          board={selectedBoard}
          source={source}
          config={activeSimulationConfig}
          onRestore={(s) => {
            const workspace = loadBoardWorkspace(s.board);
            setBridge(s.bridge);
            setSelectedBoard(s.board);
            setSource(s.source ?? workspace.source);
            setActiveSimulationConfig(s.config as ActiveSimulationConfig | null);
            editor.loadDiagram(workspace.diagram);
          }}
        />
        <BackgroundChipTicker />
        <MobileRunView
          selectedBoard={selectedBoard}
          editorState={editor.state}
          boardIoStates={boardIoStateMap}
          displayBuffers={simState.displayBuffers}
          uartOutput={simState.uartOutput}
          onButtonToggle={handleButtonToggle}
          onAnalogChange={handleAnalogChange}
          onUpdateAttr={(id, attrs) => editor.updateAttrs(id, attrs)}
          ntcTemperatures={ntcTemperatures}
          onNtcChange={handleNtcChange}
          simControls={simDockNode}
          onOpenProjects={() => setProjectsModalOpen(true)}
          onShare={handleShare}
          onPickLab={handlePickLab}
          bridge={bridge}
          running={running}
          onPartAttrChange={handlePartAttrChange}
          toast={toast}
          onDismissToast={() => setToast(null)}
          chips={mcuChips}
          foregroundChipId={foregroundPartId}
          onSelectChip={(id) => editor.select(id)}
          registers={registers}
          traceEntries={traceEntries}
          stackMemory={stackMemory}
          stackBase={stackBase}
        />
        {/* Branded back-link to the full lab — only in the embed (not on phones). */}
        {embed && <EmbedBadge />}
        {projectsModalNode}
      </ChipsProvider>
    );
  }

  const logicAnalyzerPart = editor.state.diagram.parts.find((part) => part.type === 'logic-analyzer');
  const studioTools = [
    {
      id: 'air-tracer',
      label: 'Air Tracer · BLE',
      description: 'Catch virtual-air frames (CRC)',
      active: showAnalyzer,
      onToggle: () => setShowAnalyzer((v) => !v),
    },
    {
      id: 'logic-analyzer',
      label: 'Logic Analyzer',
      description: 'Drop probe channels onto signal wires',
      active: !!logicAnalyzerPart,
      onToggle: openLogicAnalyzerTool,
    },
  ];

  return (
    <ChipsProvider initialBoard={selectedBoard}>
    <AddMcuRefSync addMcuRef={addMcuRef} />
    <SetPropsOpenRefSync setPropsOpenRef={setPropsOpenRef} />
    <ChipBridgeSync
      bridge={bridge}
      board={selectedBoard}
      source={source}
      config={activeSimulationConfig}
      onRestore={(s) => {
        // Restore the target MCU's state into App on focus switch.
        // loadDiagram reapplies the board's diagram so the visible
        // workspace updates too (selectedBoard alone doesn't).
        const workspace = loadBoardWorkspace(s.board);
        setBridge(s.bridge);
        setSelectedBoard(s.board);
        setSource(s.source ?? workspace.source);
        setActiveSimulationConfig(s.config as ActiveSimulationConfig | null);
        editor.loadDiagram(workspace.diagram);
      }}
    />
    <BackgroundChipTicker />
    <StudioShell
      boardName={activeProjectName ?? selectedBoard.name}
      isEmpty={isEmpty}
      onPickLab={handlePickLab}
      tools={studioTools}
      // Upload now lives per-chip in ChipControls — a global top-bar
      // Upload is ambiguous about which chip it targets, so it's gone.
      onShare={handleShare}
      onEmbed={handleEmbed}
      toast={toast}
      onDismissToast={() => setToast(null)}
      paletteComponents={paletteComponents}
      onPaletteDrag={handlePaletteDrag}
      inspector={null}
      // Desktop: Run/Pause/PC/cycles live inside each chip's window. Mobile
      // keeps the standalone dock.
      simDock={isMobile ? simDockNode : null}
      authSlot={<AuthPill onOpenProjects={() => setProjectsModalOpen(true)} />}
      projectSlot={
        <button
          type="button"
          onClick={() => setProjectsModalOpen(true)}
          aria-label="Open My Projects"
          title={activeProjectName ? `Open ${activeProjectName}` : 'Open My Projects'}
          className="h-7 px-3 rounded-pill text-xs font-medium bg-white/[0.05] text-fg-secondary hover:bg-white/[0.10] hover:text-fg-primary transition-colors duration-micro border-0 outline-none focus-visible:ring-2 focus-visible:ring-accent/50 flex items-center gap-1.5 shrink-0 max-w-[18ch]"
        >
          <span aria-hidden>📂</span>
          <span className="truncate">{activeProjectName ?? 'My Projects'}</span>
        </button>
      }
      renderDevDrawer={renderDevDrawer}
      renderCommandPalette={renderCommandPalette}
      onMountCommandRef={(refs) => { commandRefs.current = refs; }}
      devMode={showCode}
    >
    <AccountPanel open={accountOpen} onClose={() => setAccountOpen(false)} />
    <div data-legacy-shell="true" className={`playground${showCode ? ' code-open' : ''}`}>
      {/* ===== Unified Layout =====
          The editor SURFACE (palette · [code|canvas] · property panel) is the
          shared <LabwiredEditor> from @labwired/ui — the SAME component proto.cat
          mounts, so the two can't drift. The playground keeps its own RUNTIME
          (compile, bridges, multi-MCU) and its richer floating windows (rendered
          below), so it passes renderWindows={false} and opens windows from onSelect. */}
      <div className="editor-layout">
        <LabwiredEditor
          state={editor.state}
          // Run-only embed: a `?embed=true` pane shows a live, interactive sim
          // on docs pages — never for editing. Lock to read-only run mode so
          // viewers can't rewire/drag/delete (the Run control still works).
          interactionMode={embed ? 'run' : 'edit'}
          boardIoStates={boardIoStateMap}
          displayBuffers={simState.displayBuffers}
          validationMessage={canvasValidationMessage}
          invalidPins={invalidPins}
          onMovePart={editor.movePart}
          onResizePart={editor.resizePart}
          onSelect={(id, add) => {
            editor.select(id, add);
            // Every component opens its own floating window when clicked (chips
            // get the rich inspector, other parts their properties). Clicking
            // empty canvas (id null) or a wire opens nothing.
            if (id && editor.state.diagram.parts.some((p) => p.id === id)) {
              openWindow(id);
            }
          }}
          onSelectRect={editor.selectRect}
          onStartWire={handleStartWire}
          onCompleteWire={handleCompleteWire}
          onCancelWire={handleCancelWire}
          onDeleteWire={editor.deleteWire}
          onDropPart={handleDropPart}
          onButtonToggle={handleButtonToggle}
          onAnalogChange={handleAnalogChange}
          // Anchored control toolbar — only for MCU parts (a chip's intrinsic
          // verbs live next to it on the canvas).
          selectedPartOverlay={(part) => {
            if (!mcuBoardForPart(part, selectedBoard)) return null;
            return renderChipControls(part.id === 'mcu', 'toolbar');
          }}
          // Quiet "About this lab" affordance, preserved through the shared
          // editor's free-form overlay slot.
          overlays={selectedBoard.description ? (
            <LabInfoButton
              name={selectedBoard.name}
              description={selectedBoard.description}
              runHint={selectedBoard.runHint}
            />
          ) : undefined}
          codePane={showCode ? { source, onChange: setSource, errors: compileErrors } : false}
          propertyPanel={showRightSidebar}
          propertyLabWidget={inspectorLabWidget}
          onSetPropertyPanel={setShowRightSidebar}
          onAttrChange={(partId, key, value) => handlePartAttrChange(partId, { [key]: value })}
          onDeleteSelected={editor.deleteSelected}
          onRotatePart={editor.rotatePart}
          renderWindows={false}
        />
      </div>

      {/* ===== Loading overlay (on top of UI, not replacing it) ===== */}
      {loading && (
        <div className="loading-overlay">
          <div className="spinner" />
          <span>{compiling ? 'Compiling...' : 'Loading simulator engine...'}</span>
        </div>
      )}
    </div>
    {projectsModalNode}
    {/* One floating property window per clicked component — draggable and
        arrangeable. Chips get the rich inspector (control surface + tabs);
        other parts get their properties. Clicking a window focuses its part. */}
    {!isMobile && openWindowIds.map((w, i) => {
      const partId = w.id;
      const part = editor.state.diagram.parts.find((p) => p.id === partId);
      if (!part) return null;
      const chipBoard = mcuBoardForPart(part, selectedBoard);
      const isFg = partId === foregroundPartId;
      const dot = `h-2 w-2 shrink-0 rounded-full ${isFg ? 'bg-green-400' : 'bg-green-400/60'}`;

      if (chipBoard) {
        // Focused chip: control surface + the Run/Pause/PC/cycles readout
        // (this is the old middle dock, now living in the chip's window).
        const readout = isFg ? (
          <div className="ml-1 flex items-center gap-2 font-mono text-[10px] text-fg-tertiary">
            <button
              type="button"
              onClick={() => requireAuth(handleStep)}
              disabled={!bridge}
              title="Step one instruction"
              className="inline-flex h-6 w-6 items-center justify-center rounded-md border border-border text-fg-secondary hover:bg-bg-elevated hover:text-fg-primary disabled:opacity-40"
            >
              ⏵
            </button>
            <span title="Time this MCU has been running">⏱ {formatRuntime(runtimeMs)}</span>
            <span>{(simState.cycles ?? 0).toLocaleString()} cyc</span>
            <span>PC 0x{(simState.pc ?? 0).toString(16).toUpperCase()}</span>
          </div>
        ) : null;
        return (
          <ChipWindow
            key={partId}
            initial={{ x: w.x, y: w.y }}
            width={500}
            height={380}
            zIndex={Math.min(60 + i, 95)}
            onFocus={() => editor.select(partId)}
            onClose={() => closeWindow(partId)}
            title={
              <>
                <span className={dot} />
                <span className="truncate font-mono text-xs text-fg-primary">{chipBoard.name}</span>
                {isFg && <span className="shrink-0 text-[10px] text-accent">focused</span>}
              </>
            }
          >
            <ChipInspector
              board={chipBoard}
              isForeground={isFg}
              hasLiveSim={!!bridge}
              controls={isFg ? <>{renderChipControls(partId === 'mcu', 'header')}{readout}</> : undefined}
              actions={
                <PartActions
                  onRotate={() => editor.rotatePart(partId)}
                  scale={part.scale ?? 1}
                  onScale={(s) => editor.resizePart(partId, s)}
                  onDelete={() => { editor.select(partId); editor.deleteSelected(); closeWindow(partId); }}
                  canDelete={partId !== 'mcu'}
                />
              }
              serialOutput={chipSerialFor(partId, isFg)}
              onClearSerial={() => clearChipSerial(partId)}
              registers={registers}
              traceEntries={traceEntries}
              stackMemory={stackMemory}
              stackBase={stackBase}
            />
          </ChipWindow>
        );
      }

      if (part.type === 'logic-analyzer') {
        return (
          <ChipWindow
            key={partId}
            initial={{ x: w.x, y: w.y }}
            width={580}
            height={380}
            zIndex={Math.min(60 + i, 95)}
            onFocus={() => editor.select(partId)}
            onClose={() => closeWindow(partId)}
            title={
              <span className="truncate text-xs font-semibold text-fg-primary">Logic Analyzer</span>
            }
          >
            <LogicAnalyzerPanel
              diagram={editor.state.diagram}
              analyzerId={partId}
              bridge={bridge}
              running={running}
              decoder={part.attrs.decoder ?? 'raw'}
              onDecoderChange={(decoder) => handlePartAttrChange(partId, { decoder })}
            />
          </ChipWindow>
        );
      }

      // Non-chip component → its own meaningful, editable properties.
      const def = COMPONENT_REGISTRY.get(part.type);
      const live = boardIoStateMap[partId];
      return (
        <ChipWindow
          key={partId}
          initial={{ x: w.x, y: w.y }}
          width={300}
          height={280}
          zIndex={Math.min(60 + i, 95)}
          onFocus={() => editor.select(partId)}
          onClose={() => closeWindow(partId)}
          title={
            <span className="truncate font-mono text-xs text-fg-primary">{def?.label ?? part.type}</span>
          }
        >
          <ComponentInspector
            partType={part.type}
            partId={partId}
            attrs={part.attrs ?? {}}
            fields={def?.attrFields ?? []}
            live={live ? { active: live.active, analogValue: live.analogValue } : undefined}
            onChange={(key, value) => handlePartAttrChange(partId, { [key]: value })}
            runtimeControl={renderRuntimeControl(part)}
            actions={
              <PartActions
                onRotate={() => editor.rotatePart(partId)}
                scale={part.scale ?? 1}
                onScale={(s) => editor.resizePart(partId, s)}
                onDelete={() => { editor.select(partId); editor.deleteSelected(); closeWindow(partId); }}
              />
            }
          />
        </ChipWindow>
      );
    })}

    {/* Packet Analyzer — opt-in Tools instrument, toggled from the SimDock.
        Uses the shared ChipWindow chrome so it's movable/resizable/closable
        with the same controls as every chip window on the canvas. */}
    {!isMobile && showAnalyzer && (
      <ChipWindow
        initial={{ x: 900, y: 420 }}
        width={520}
        height={320}
        zIndex={95}
        onClose={() => setShowAnalyzer(false)}
        title={
          <span className="truncate text-xs font-semibold text-fg-primary">
            Air Tracer · virtual wireless (BLE)
          </span>
        }
      >
        <BleAnalyzer bridge={bridge} running={running} />
      </ChipWindow>
    )}
    {!isMobile && showIolink && (
      <ChipWindow
        initial={{ x: 900, y: 120 }}
        width={540}
        height={360}
        zIndex={95}
        onClose={() => setShowIolink(false)}
        title={
          <span className="truncate text-xs font-semibold text-fg-primary">
            IO-Link Analyzer · master↔device
          </span>
        }
      >
        <IoLinkAnalyzer bridge={bridge} running={running} />
      </ChipWindow>
    )}
    <EmbedDialog
      open={embedOpen}
      onClose={() => setEmbedOpen(false)}
      diagram={editor.state.diagram}
      source={source}
      buildExtras={buildShareExtras}
      onError={(message) => setToast(message)}
    />
    {/* Branded attribution shown only inside an embedded (?embed=true) lab. */}
    {embed && <EmbedBadge />}
    </StudioShell>
    </ChipsProvider>
  );
}

function BackgroundChipTicker() {
  useBackgroundChips(true);
  return null;
}

/// Bridges the ChipsProvider's addChip to the parent App scope via a
/// ref so the command palette (defined outside the provider) can drop
/// a new MCU through the standard ⌘K flow.
export function AddMcuRefSync({
  addMcuRef,
}: {
  addMcuRef: { current: ((board?: BoardConfig) => void) | null };
}) {
  const chips = useChips();
  addMcuRef.current = (board) => {
    chips.addChip(board);
  };
  return null;
}

/// Bridges setPropertiesOpen out of the provider so the EditorCanvas
/// MCU click can open the drawer.
export function SetPropsOpenRefSync({
  setPropsOpenRef,
}: {
  setPropsOpenRef: { current: ((open: boolean) => void) | null };
}) {
  const chips = useChips();
  setPropsOpenRef.current = chips.setPropertiesOpen;
  return null;
}
