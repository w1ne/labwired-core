import { useState, useCallback, useRef, useMemo, useEffect, type ReactNode } from 'react';
import { ProjectsModal } from './studio/ProjectsModal';
import type { ProjectRecord } from './studio/useProjects';
import { CommandPalette } from './studio/CommandPalette';
import { useCommandPaletteItems } from './studio/useCommandPaletteItems';
import {
  SimControls,
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
  EditorCanvas,
  ComponentPalette,
  PropertyPanel,
  CodeEditor,
  compileCode,
  EXAMPLE_SKETCHES,
  useEditorState,
  diagramToConfig,
  validateDiagram,
  validateWireConnection,
  COMPONENT_REGISTRY,
  createEmptyDiagram,
  decodeProject,
  generateShareUrl,
  isEmbedMode,
  type CompileError,
  type TraceEntry,
  type WasmModule,
  type Part,
  type Diagram,
  type BoardIoBinding,
  type ComponentState,
} from '@labwired/ui';
import { BOARD_CONFIGS, type BoardConfig } from './bundled-configs';
import { resolveBoardForPart } from './board-resolve';
import { fetchCatalog, type CatalogEntry } from './catalog-client';
import { useUser, useClerk } from '@clerk/clerk-react';
import { StudioShell } from './studio/StudioShell';
import { ChipsProvider, useChips } from './multi-mcu/ChipsProvider';
import { ChipBridgeSync } from './multi-mcu/ChipBridgeSync';
import { useBackgroundChips } from './multi-mcu/useBackgroundChips';
import { MobileMultiChipView } from './multi-mcu/MobileMultiChipView';
import { PropertiesGate } from './multi-mcu/PropertiesGate';
import { ChipTabsBar, DrawerCloseButton } from './multi-mcu/ChipTabsBar';
import { ChipControls } from './multi-mcu/ChipControls';
import { usePerChipSims } from './multi-mcu/usePerChipSims';
import { ChipWindow } from './multi-mcu/ChipWindow';
import { ChipInspector } from './multi-mcu/ChipInspector';
import { McuStrip } from './multi-mcu/McuStrip';
import { BleAnalyzer } from './instruments/BleAnalyzer';
import { ToolsMenu } from './studio/ToolsMenu';
import { AuthPill } from './studio/AuthPill';
import { getComponentIcon } from './studio/componentIcons';
import { WatchOverlay } from './studio/WatchOverlay';
import { AccountPanel } from './studio/AccountPanel';
import { DevDrawer } from './studio/DevDrawer';
import { SimDock, type SimState as StudioSimState } from './studio/SimDock';
import { type InspectorSelection } from './studio/InspectorCard';
import { ComponentInspector } from './multi-mcu/ComponentInspector';
import { PartActions } from './multi-mcu/PartActions';
import { type PaletteComponent, type PaletteCategory } from './studio/PaletteDrawer';
import { BoardPicker } from './BoardPicker';
import {
  CheckIcon, UploadIcon, CodeIcon, PanelBottomIcon,
  ShareIcon, ExportIcon, ImportIcon, UndoIcon, RedoIcon,
  StopIcon, SidebarLeftIcon, SidebarRightIcon,
  ChevronLeftIcon, ChevronRightIcon,
} from './Icons';

type BottomTab = 'output' | 'serial' | 'registers' | 'trace' | 'memory';
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

function makeStarterDiagram(config: BoardConfig): Diagram {
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
    return {
      ...createEmptyDiagram(config.chipId),
      parts: [
        { id: 'mcu', type: config.mcuComponentType, x: 100, y: 160, rotate: 0,
          attrs: { boardId: 'nrf52840-ble-sensor' } },
        { id: 'mcu-collector', type: config.mcuComponentType, x: 560, y: 160, rotate: 0,
          attrs: { boardId: 'nrf52840-ble-collector' } },
      ],
      wires: [],
    };
  }

  if (config.boardId === 'stm32f103-blinky') {
    return {
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
    };
  }

  // -------- I²C labs (oled, sensors): all share PB6 SCL / PB7 SDA on I2C1 --------

  if (config.boardId === 'ssd1306-hello-lab') {
    return {
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
    };
  }

  if (config.boardId === 'nokia5110-invaders-lab') {
    // STM32L476 Breakout: Nokia 5110 (PCD8544 SPI1) + HC-SR04. Part ids match
    // the lab's external_device ids ('lcd', 'dist') so the framebuffer fetch
    // and distance setter resolve.
    return {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'lcd', type: 'pcd8544', x: 500, y: 60, rotate: 0, attrs: {} },
        { id: 'dist', type: 'ultrasonic', x: 500, y: 280, rotate: 0, attrs: {} },
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
    };
  }

  if (config.boardId === 'mpu6050-sensor-lab') {
    return {
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
    };
  }

  if (config.boardId === 'adxl345-sensor-lab') {
    return {
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
    };
  }

  if (config.boardId === 'bme280-weather-lab') {
    return {
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
    };
  }

  // -------- SPI labs --------

  if (config.boardId === 'ili9341-tft-lab') {
    // ILI9341 sim ignores D/C (state machine over command boundaries), but
    // real hardware needs it — wire to PB0 so the same diagram is honest for
    // both. RESET wired to PB1; LED backlight tied to VCC.
    return {
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
    };
  }

  if (config.boardId === 'max31855-thermocouple-lab') {
    return {
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
    };
  }

  // -------- UART --------

  if (config.boardId === 'neo6m-gps-lab') {
    // STM32 TX → GPS RX, GPS TX → STM32 RX (crossover).
    return {
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
    };
  }

  // -------- Analog (ADC) --------

  if (config.boardId === 'ntc-thermistor-lab') {
    // NTC voltage divider sits between VCC and GND; tap into ADC1 ch0 on PA0.
    return {
      ...createEmptyDiagram(config.chipId),
      parts: [
        mcu,
        { id: 'ntc', type: 'ntc-thermistor', x: 540, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PA0' }, to: { part: 'ntc', pin: 'A' }, color: '#F5B642' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'ntc', pin: 'B' }, color: '#888888' },
      ],
    };
  }

  if (config.boardId === 'epaper-tricolor-lab') {
    // STM32F103 driving the Waveshare 2.9" SSD1680 tri-color panel.
    // Pin map matches the firmware (examples/epaper-tricolor-lab/src/main.rs)
    // AND a real NUCLEO-F103RB wiring of the panel — same ELF runs in both.
    return {
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
    };
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
    return {
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
    };
  }

  if (config.boardId === 'nucleo-f401re') {
    return {
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
    };
  }

  return {
    ...createEmptyDiagram(config.chipId),
    parts: [mcu],
    wires: [],
  };
}

function getDefaultSource(config: BoardConfig): string {
  if (config.boardId === 'nucleo-f401re') {
    return EXAMPLE_SKETCHES.find((sketch) => sketch.name === 'Button + LED')?.source ?? EXAMPLE_SKETCHES[0].source;
  }
  return EXAMPLE_SKETCHES.find((sketch) => sketch.name === 'Blink')?.source ?? EXAMPLE_SKETCHES[0].source;
}

function loadBoardWorkspace(config: BoardConfig): { diagram: Diagram; source: string } {
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

const PALETTE_CATEGORY: Record<string, PaletteCategory> = {
  adxl345: 'i2c',
  bme280: 'i2c',
  ili9341: 'spi',
  max31855: 'spi',
  mpu6050: 'i2c',
  'oled-ssd1306': 'i2c',
  'neo6m-gps': 'uart',
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
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const traceRef = useRef<TraceEntry[]>([]);
  const [traceEntries, setTraceEntries] = useState<TraceEntry[]>([]);
  const [canvasValidationMessage, setCanvasValidationMessage] = useState<string | null>(null);
  const [invalidPins, setInvalidPins] = useState<Array<{ part: string; pin: string }>>([]);

  // Board selection (from catalog + bundled configs)
  const [catalog, setCatalog] = useState<CatalogEntry[]>([]);
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
  const [compileOutput, setCompileOutput] = useState('');
  const [compiling, setCompiling] = useState(false);
  const [bottomTab, setBottomTab] = useState<BottomTab>('output');
  const [showCode, setShowCode] = useState(false);
  const [showBottomPanel, setShowBottomPanel] = useState(true);
  const [showLeftSidebar, setShowLeftSidebar] = useState(true);
  const [projectsModalOpen, setProjectsModalOpen] = useState(false);
  // Tracks the currently-loaded cloud project (null if the canvas is from
  // a board starter or unsaved). When set, "Save" overwrites this project.
  const [activeProjectId, setActiveProjectId] = useState<string | null>(null);
  const [activeProjectName, setActiveProjectName] = useState<string | null>(null);
  const [showRightSidebar, setShowRightSidebar] = useState(true);
  // Analyzer is an opt-in instrument now (was an always-on panel that froze the
  // canvas). Toggled from the SimDock Tools control; hidden by default.
  const [showAnalyzer, setShowAnalyzer] = useState(false);
  const embed = isEmbedMode();
  const autostartTriggeredRef = useRef(false);

  // Command palette mode + ref for global ⌘K shortcut
  const commandRefs = useRef<{ open: () => void; close: () => void } | null>(null);

  // Editor state
  const editor = useEditorState(
    loadBoardWorkspace(selectedBoard).diagram,
  );

  // Whether simulation has been loaded (bridge exists)
  const simActive = !!bridge;

  // Fetch catalog on mount
  useEffect(() => {
    fetchCatalog().then(setCatalog);
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
    await mod.default();
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
        const resp = await fetch(config.bootSnapshotUrl);
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
    setBottomTab('serial');
    setShowBottomPanel(true);
  }, [loadWasm]);

  // Compile source code
  const handleCompile = useCallback(async () => {
    const diagramErrors = validateDiagram(editor.state.diagram);
    if (diagramErrors.length > 0) {
      setCanvasValidationMessage(diagramErrors[0]);
      setInvalidPins([]);
      setCompileErrors([]);
      setCompileOutput(`Wiring error: ${diagramErrors[0]}`);
      setBottomTab('output');
      setShowBottomPanel(true);
      return null;
    }

    setCanvasValidationMessage(null);
    setInvalidPins([]);
    setCompiling(true);
    setCompileOutput('Compiling...\n');
    setCompileErrors([]);
    setBottomTab('output');
    setShowBottomPanel(true);
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

      if (result?.success && result.elf) {
        firmware = result.elf;
        // Use diagram-derived config with the run board's chip YAML
        const config = diagramToConfig(editor.state.diagram, runBoard.chipYaml);
        systemYaml = config.systemYaml;
        chipYaml = config.chipYaml;
        setCompileOutput((prev) => prev + '\nUpload successful. Starting simulation...');
      } else if (runBoard.demoFirmwarePath) {
        // Fall back to pre-built demo firmware with its matching YAML configs
        const resp = await fetch(runBoard.demoFirmwarePath);
        if (!resp.ok) throw new Error(`Failed to load firmware: ${runBoard.demoFirmwarePath}`);
        firmware = new Uint8Array(await resp.arrayBuffer());
        systemYaml = runBoard.systemYaml;
        chipYaml = runBoard.chipYaml;
        setCompileOutput((prev) => prev + '\nUsing pre-built demo firmware.');
      } else {
        // No demo firmware and compile failed
        setCompileOutput(
          (prev) => prev + `\nNo pre-built firmware for ${runBoard.name}. Write code and compile it first.`,
        );
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

  // Stop simulation
  const handleStop = useCallback(() => {
    setRunning(false);
    setBridge(null);
  }, []);

  // Build the display-device list from the diagram so the loop knows what
  // to poll. Filter to types that have a known wasm framebuffer accessor.
  const displays = useMemo(
    () =>
      editor.state.diagram.parts
        .filter(
          (p) =>
            p.type === 'ssd1680_tricolor_290' ||
            p.type === 'uc8151d_tricolor_290',
        )
        .map((p) => ({
          partId: p.id,
          kind: p.type as 'ssd1680_tricolor_290' | 'uc8151d_tricolor_290',
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
      setBottomTab('output');
      setShowBottomPanel(true);
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
    const id = window.setInterval(poll, 100);
    return () => window.clearInterval(id);
  }, [running, bridge]);

  // PCD8544 (Nokia 5110) live framebuffer + HC-SR04 hand distance
  const [pcd8544Framebuffer, setPcd8544Framebuffer] = useState<Uint8Array | null>(null);
  const [hcsr04Distance, setHcsr04DistanceState] = useState(30);

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
    const id = window.setInterval(poll, 100);
    return () => window.clearInterval(id);
  }, [running, bridge]);

  const handleDistanceChange = useCallback(
    (cm: number) => {
      setHcsr04DistanceState(cm);
      bridge?.setHcsr04Distance('dist', cm);
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
    const id = window.setInterval(poll, 100);
    return () => window.clearInterval(id);
  }, [running, bridge]);

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

  // Editor: add part from palette
  const handleAddPartFromPalette = useCallback(
    (type: string) => {
      const def = COMPONENT_REGISTRY.get(type);
      if (!def) return;
      const part: Part = {
        id: nextPartId(type), type, x: 400, y: 200, rotate: 0,
        attrs: { ...def.defaultAttrs },
      };
      editor.addPart(part);
    },
    [editor],
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

  // Build live sensor widget for selected I2C / UART devices
  const inspectorLabWidget = useMemo<ReactNode>(() => {
    if (!bridge || !inspectorSelection || inspectorSelection.kind !== 'part') return undefined;
    const partType = inspectorSelection.partType;
    if (partType === 'oled-ssd1306') {
      return <Ssd1306Display framebuffer={ssd1306Framebuffer} width={256} />;
    }
    if (partType === 'pcd8544' || partType === 'nokia-5110') {
      return (
        <div className="flex flex-col gap-3">
          <Pcd8544Display framebuffer={pcd8544Framebuffer} width={252} />
          <label className="block">
            <div className="flex items-center justify-between text-fg-tertiary text-[11px] font-mono mb-1">
              <span>HC-SR04 hand distance</span>
              <span className="text-fg-primary">{hcsr04Distance.toFixed(0)} cm</span>
            </div>
            <input
              type="range"
              min={2}
              max={200}
              step={1}
              value={hcsr04Distance}
              onChange={(e) => handleDistanceChange(parseFloat(e.target.value))}
              style={{ width: '100%' }}
            />
          </label>
        </div>
      );
    }
    if (partType === 'ili9341') {
      return <Ili9341Display framebuffer={ili9341Framebuffer} width={240} />;
    }
    if (partType === 'neo6m-gps') {
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
    if (partType !== 'adxl345' && partType !== 'mpu6050' && partType !== 'bme280') return undefined;
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
  }, [bridge, inspectorSelection, simState.pc, ssd1306Framebuffer, pcd8544Framebuffer, hcsr04Distance, handleDistanceChange, ili9341Framebuffer, adcDeviceStates, ntcTemperatures]);

  // Right-side InspectorCard removed — Properties live in the
  // bottom drawer (per-chip Serial/Registers/Trace/Memory/Source/
  // YAML). The part-specific inspector (lab widgets, delete/
  // duplicate) is dropped along with it; can be reintroduced as a
  // tab on the properties drawer if useful later.
  void inspectorSelection;
  void inspectorLabWidget;

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
        let systemYaml = target.systemYaml;
        let chipYaml = target.chipYaml;
        if (drawerSubject.isPrimary) {
          try {
            const config = diagramToConfig(editor.state.diagram, target.chipYaml);
            systemYaml = config.systemYaml;
            chipYaml = config.chipYaml;
          } catch (configErr) {
            const msg = configErr instanceof Error ? configErr.message : String(configErr);
            setCompileOutput((prev) => `${prev}\nUsing bundled system YAML — canvas not used: ${msg}`);
          }
        }

        await launchSimulation({
          systemYaml,
          chipYaml,
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

  const selectedParts = editor.state.diagram.parts.filter((p) => editor.state.selectedIds.has(p.id));
  const currentExample = EXAMPLE_SKETCHES.find((sketch) => sketch.source === source) ?? null;
  const boardSummary = useMemo(() => {
    const componentCount = Math.max(editor.state.diagram.parts.length - 1, 0);
    const wireCount = editor.state.diagram.wires.length;
    // Boards may carry a hand-crafted summary in BoardConfig — use it
    // verbatim. `nextStepRunning` (optional) is swapped in when the sim
    // is active. Falls through to a generic "Click Run" hint otherwise.
    if (selectedBoard.summary) {
      const s = selectedBoard.summary;
      return {
        title: s.title,
        description: s.description,
        nextStep: simActive ? (s.nextStepRunning ?? s.nextStep) : s.nextStep,
      };
    }
    if (selectedBoard.boardId === 'stm32f103-blinky') {
      return {
        title: 'STM32 demo circuit',
        description: 'PA5 drives the onboard LED. Upload runs the bundled blinky immediately.',
        nextStep: simActive ? 'Simulation is running. Watch the LED state and serial pane.' : 'Click Run Demo to boot the bundled STM32 blinky.',
      };
    }
    if (selectedBoard.boardId === 'nucleo-f401re') {
      return {
        title: 'LED + button starter',
        description: 'PA5 drives the LED and PC13 is wired to the user button.',
        nextStep: simActive ? 'Simulation is running. Press the button component to interact.' : 'Click Run Demo to boot the starter circuit.',
      };
    }
    return {
      title: `${selectedBoard.name} starter`,
      description: `${componentCount} components and ${wireCount} wires on the canvas.`,
      nextStep: selectedBoard.demoFirmwarePath
        ? 'Click Run Demo to boot the bundled firmware.'
        : 'Wire a circuit, compile the sketch, then run it.',
    };
  }, [editor.state.diagram.parts.length, editor.state.diagram.wires.length, selectedBoard, simActive]);

  // Load from URL hash (sharing) or localStorage
  useEffect(() => {
    const hash = window.location.hash.slice(1);
    if (hash) {
      decodeProject(hash).then((project) => {
        if (project) {
          editor.loadDiagram(project.diagram);
          setSource(project.source);
          for (const p of project.diagram.parts) {
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

  useEffect(() => {
    localStorage.setItem(
      getWorkspaceStorageKey(selectedBoard.boardId, 'diagram'),
      JSON.stringify(editor.state.diagram),
    );
  }, [editor.state.diagram, selectedBoard.boardId]);

  useEffect(() => {
    localStorage.setItem(getWorkspaceStorageKey(selectedBoard.boardId, 'source'), source);
  }, [source, selectedBoard.boardId]);

  // Export/Import
  const handleExport = useCallback(() => {
    const data = { diagram: editor.state.diagram, source };
    const json = JSON.stringify(data, null, 2);
    const blob = new Blob([json], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url; a.download = 'project.json'; a.click();
    URL.revokeObjectURL(url);
  }, [editor.state.diagram, source]);

  const handleImport = useCallback(() => {
    const input = document.createElement('input');
    input.type = 'file'; input.accept = '.json';
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return;
      const text = await file.text();
      try {
        const data = JSON.parse(text);
        if (data.diagram) {
          editor.loadDiagram(data.diagram as Diagram);
          if (data.source) setSource(data.source);
        } else {
          editor.loadDiagram(data as Diagram);
        }
      } catch { alert('Invalid project file'); }
    };
    input.click();
  }, [editor]);

  const handleResetDemo = useCallback(() => {
    const starter = makeStarterDiagram(selectedBoard);
    localStorage.removeItem(getWorkspaceStorageKey(selectedBoard.boardId, 'diagram'));
    localStorage.removeItem(getWorkspaceStorageKey(selectedBoard.boardId, 'source'));
    editor.loadDiagram(starter);
    setSource(getDefaultSource(selectedBoard));
    setCanvasValidationMessage(null);
    setInvalidPins([]);
    setCompileErrors([]);
    setCompileOutput(`Restored ${selectedBoard.name} demo workspace.`);
    setBottomTab('output');
    setShowBottomPanel(true);
    setRunning(false);
    setBridge(null);
    setActiveSimulationConfig(null);
  }, [editor, selectedBoard]);

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
  const { openSignIn } = useClerk();
  const requireAuth = useCallback(
    (action: () => void) => {
      // Local-dev escape hatch: set VITE_DISABLE_AUTH=true in
      // packages/playground/.env.local to run sims without signing in.
      // Off by default, so production (Cloudflare Pages) stays gated.
      if (import.meta.env.VITE_DISABLE_AUTH === 'true') {
        action();
        return;
      }
      if (clerkLoaded && !isSignedIn) {
        openSignIn({});
        return;
      }
      action();
    },
    [clerkLoaded, isSignedIn, openSignIn],
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
      const url = await generateShareUrl(editor.state.diagram, source);
      await navigator.clipboard.writeText(url);
      setToast('Share URL copied to clipboard');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setToast(`Share failed: ${message}`);
    }
  }, [editor.state.diagram, source]);

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

  // Bottom tab content
  const bottomContent = () => {
    switch (bottomTab) {
      case 'output':
        return <pre className="compile-output">{compileOutput || 'Ready to compile.'}</pre>;
      case 'serial':
        return <SerialMonitor output={simState.uartOutput} onClear={clearUart} style={{ height: '100%' }} />;
      case 'registers':
        return <RegisterGrid registers={registers} style={{ maxHeight: '100%', overflow: 'auto' }} />;
      case 'trace':
        return <InstructionTrace entries={traceEntries} style={{ maxHeight: '100%', overflow: 'auto' }} />;
      case 'memory':
        return <MemoryInspector data={stackMemory} baseAddress={stackBase} style={{ maxHeight: '100%', overflow: 'auto' }} />;
    }
  };

  // Bridge so the command palette (defined here, outside the
  // ChipsProvider tree) can drop a new MCU through the ⌘K flow.
  const addMcuRef = useRef<((board?: BoardConfig) => void) | null>(null);
  // Bridge to open the properties drawer when the user clicks the
  // MCU part on the EditorCanvas — chip-on-canvas IS the affordance.
  const setPropsOpenRef = useRef<((open: boolean) => void) | null>(null);

  // Command palette items
  const commandItems = useCommandPaletteItems({
    boards: BOARD_CONFIGS.filter((b) => !b.hidden),
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
    const canRun = isPrimary || !!activeSimulationConfig;
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
          // Narrow viewports get a wrapped two-line pill instead of overflowing.
          className="max-w-[92vw] sm:max-w-none px-3 py-1.5 rounded-2xl sm:rounded-pill bg-accent/15 border border-accent/40 text-accent text-[11px] font-medium flex items-start sm:items-center gap-1.5 shadow-[0_6px_18px_-6px_rgba(91,157,255,0.45)] text-center sm:text-left leading-snug"
        >
          <span aria-hidden className="mt-0.5 sm:mt-0">▶</span>
          <span className="break-words">
            {selectedBoard.runHint
              ?? (selectedBoard.kind === 'lab'
                ? 'Click Run to start the simulation'
                : 'Click Run to start — the LED should blink')}
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

  // Mobile-only demo shell: the desktop canvas editor is unusable on a phone.
  // Render a focused single-screen view (board name → big e-paper preview →
  // big Run button) below the md breakpoint instead of squeezing the editor.
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

  if (isMobile) {
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
        <MobileMultiChipView
          propertiesContent={renderDevDrawer?.(true, 0)}
          simControls={simDockNode}
          uartPreview={simState.uartOutput}
          running={running}
          cyclesActive={simState.cycles ?? 0}
          canRun={!!source || !!bridge}
          onRun={() => requireAuth(onSimRun)}
          onPause={handlePause}
          renderCommandPalette={renderCommandPalette}
        />
      </ChipsProvider>
    );
  }

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
      // Upload now lives per-chip in ChipControls — a global top-bar
      // Upload is ambiguous about which chip it targets, so it's gone.
      onShare={handleShare}
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
    {/* Multi-MCU switcher — one tile per chip in the session. Self-hides for a
        single chip; appears once a second board is added (⌘K → pick a board),
        letting you switch the active/foreground chip. This is what surfaces the
        two-board comms demo on desktop. */}
    {!embed && <McuStrip />}
    <div data-legacy-shell="true" className={`playground${showCode ? ' code-open' : ''}`}>
      {/* ===== Header ===== */}
      {!embed && (
        <div className="playground-header">
          {/* --- Project group --- */}
          <div className="toolbar-group">
            <span className="logo">LabWired</span>
            <BoardPicker
              catalog={catalog}
              selectedBoardId={selectedBoard.boardId}
              onSelect={(cfg) => {
                // Switching board breaks the link to the loaded project.
                setActiveProjectId(null);
                setActiveProjectName(null);
                handleBoardSelect(cfg);
              }}
            />
            <button
              className="project-selector"
              onClick={() => setProjectsModalOpen(true)}
              title="Open My Projects"
            >
              {activeProjectName ? `📂 ${activeProjectName}` : '📂 My Projects'}
            </button>
            <select
              className="project-selector"
              value={currentExample?.name ?? ''}
              onChange={(e) => {
                const sketch = EXAMPLE_SKETCHES.find((s) => s.name === e.target.value);
                if (sketch) setSource(sketch.source);
              }}
            >
              <option value="" disabled>Examples...</option>
              {EXAMPLE_SKETCHES.map((s) => (
                <option key={s.name} value={s.name}>{s.name}</option>
              ))}
            </select>
          </div>

          <div className="header-separator" />

          {/* --- Build group --- */}
          <div className="toolbar-group">
            <button className="toolbar-btn toolbar-btn-primary toolbar-btn-verify" onClick={handleCompile} disabled={compiling}>
              <CheckIcon size={14} /> {compiling ? 'Checking...' : 'Check Wiring'}
            </button>
            <button className="toolbar-btn toolbar-btn-primary" onClick={() => requireAuth(handleRun)} disabled={compiling || loading}>
              <UploadIcon size={14} /> {selectedBoard.demoFirmwarePath ? 'Run Demo' : 'Run Circuit'}
            </button>
            <button
              className="toolbar-btn toolbar-btn-primary"
              onClick={() => {
                // Self-serve path: open GitHub's "Create from template"
                // page with the lab name pre-filled. The template repo
                // (w1ne/labwired-lab-template) ships a CI workflow that
                // runs the LabWired simulator on every push — same
                // firmware that paints the panel here paints in CI too.
                const repoName = `labwired-${selectedBoard.boardId}`.toLowerCase().replace(/[^a-z0-9-]+/g, '-');
                const url = `https://github.com/new?template_owner=w1ne&template_name=labwired-lab-template&name=${encodeURIComponent(repoName)}&description=${encodeURIComponent(`${selectedBoard.name} lab gated by the LabWired simulator in CI`)}`;
                window.open(url, '_blank', 'noopener,noreferrer');
              }}
              title="Create a GitHub repo from the LabWired CI template — runs the simulator on every push as a merge gate"
            >
              <UploadIcon size={14} /> Deploy to CI
            </button>
            <button
              className="toolbar-btn toolbar-btn-ghost"
              onClick={() => {
                // Consulting path: existing repo / custom firmware
                // build / custom assertions. Books a call with the
                // LabWired team to wire CI into the user's existing
                // GitHub repo. Same Calendly link the /ci landing page
                // uses for enterprise inquiries.
                window.open('https://cal.com/andriishylenko/30min', '_blank', 'noopener,noreferrer');
              }}
              title="Already have a repo? Book 30 min — we wire LabWired CI into your existing pipeline."
            >
              Connect existing repo →
            </button>
            <button className="toolbar-btn toolbar-btn-ghost" onClick={handleResetDemo} title="Reset starter workspace">
              Reset Demo
            </button>
          </div>

          {/* --- Sim controls (inline when active) --- */}
          {simActive && (
            <>
              <div className="header-separator" />
              <div className="toolbar-group">
                <SimControls
                  variant="dark"
                  running={running}
                  onPlay={() => requireAuth(handlePlay)}
                  onPause={handlePause}
                  onStep={() => requireAuth(handleStep)}
                  onReset={handleReset}
                  pc={simState.pc}
                  cycles={simState.cycles}
                />
                <button className="toolbar-btn toolbar-btn-ghost toolbar-btn-stop" onClick={handleStop} title="Stop simulation">
                  <StopIcon size={14} />
                </button>
              </div>
            </>
          )}

          <div className="header-spacer" />

          {/* --- View group --- */}
          <div className="toolbar-group">
            <button
              className={`toolbar-btn toolbar-btn-ghost ${showCode ? 'active' : ''}`}
              onClick={() => setShowCode(!showCode)}
              title="Toggle code editor"
            >
              <CodeIcon size={14} />
            </button>
            <button
              className={`toolbar-btn toolbar-btn-ghost ${showBottomPanel ? 'active' : ''}`}
              onClick={() => setShowBottomPanel(!showBottomPanel)}
              title="Toggle bottom panel"
            >
              <PanelBottomIcon size={14} />
            </button>
            <button
              className={`toolbar-btn toolbar-btn-ghost ${showLeftSidebar ? 'active' : ''}`}
              onClick={() => setShowLeftSidebar(!showLeftSidebar)}
              title="Toggle components panel"
            >
              <SidebarLeftIcon size={14} />
            </button>
            <button
              className={`toolbar-btn toolbar-btn-ghost ${showRightSidebar ? 'active' : ''}`}
              onClick={() => setShowRightSidebar(!showRightSidebar)}
              title="Toggle properties panel"
            >
              <SidebarRightIcon size={14} />
            </button>
            <ToolsMenu
              tools={[
                {
                  id: 'air-tracer',
                  label: 'Air Tracer · BLE',
                  description: 'Catch virtual-air frames (CRC)',
                  active: showAnalyzer,
                  onToggle: () => setShowAnalyzer((v) => !v),
                },
              ]}
            />
          </div>

          <div className="header-separator" />

          {/* --- File group --- */}
          <div className="toolbar-group">
            <button className="toolbar-btn toolbar-btn-ghost" onClick={handleShare} title="Share project">
              <ShareIcon size={14} />
            </button>
            <button className="toolbar-btn toolbar-btn-ghost" onClick={handleExport} title="Export project">
              <ExportIcon size={14} />
            </button>
            <button className="toolbar-btn toolbar-btn-ghost" onClick={handleImport} title="Import project">
              <ImportIcon size={14} />
            </button>
          </div>

          <div className="header-separator" />

          {/* --- History group --- */}
          <div className="toolbar-group">
            <button
              className="toolbar-btn toolbar-btn-ghost"
              onClick={editor.undo}
              disabled={editor.state.undoStack.length === 0}
              title="Undo (Ctrl+Z)"
            >
              <UndoIcon size={14} />
            </button>
            <button
              className="toolbar-btn toolbar-btn-ghost"
              onClick={editor.redo}
              disabled={editor.state.redoStack.length === 0}
              title="Redo (Ctrl+Shift+Z)"
            >
              <RedoIcon size={14} />
            </button>
          </div>

          {error && <span className="header-error">{error}</span>}
        </div>
      )}

      {/* ===== Unified Layout ===== */}
      <div className="editor-layout">
        {/* Component palette (left sidebar) */}
        {showLeftSidebar && (
          <div className="editor-sidebar-left">
            <ComponentPalette onAddPart={handleAddPartFromPalette} />
          </div>
        )}

        {/* Collapsed left sidebar tab */}
        {!showLeftSidebar && (
          <button
            className="sidebar-toggle sidebar-toggle-left"
            onClick={() => setShowLeftSidebar(true)}
            title="Show components"
          >
            <ChevronRightIcon size={12} />
          </button>
        )}

        {/* Main content area */}
        <div className="editor-center">
          <div className="demo-banner">
            <div className="demo-banner-copy">
              <span className="demo-banner-kicker">{boardSummary.title}</span>
              <strong>{selectedBoard.name}</strong>
              <span>{boardSummary.description}</span>
              <span className="demo-banner-next">{boardSummary.nextStep}</span>
            </div>
            <div className="demo-banner-stats">
              <span className="demo-stat"><strong>{Math.max(editor.state.diagram.parts.length - 1, 0)}</strong> parts</span>
              <span className="demo-stat"><strong>{editor.state.diagram.wires.length}</strong> wires</span>
              <span className={`demo-stat ${simActive ? 'live' : ''}`}><strong>{simActive ? 'Live' : 'Idle'}</strong> sim</span>
            </div>
          </div>
          <div className="editor-top-split">
            {/* Code editor (left pane) */}
            {showCode && (
              <div className="editor-code-pane">
                <CodeEditor
                  source={source}
                  onChange={setSource}
                  errors={compileErrors}
                />
              </div>
            )}
            {/* Circuit canvas (always visible — shows live state during sim) */}
            <div className="editor-canvas-pane">
              <EditorCanvas
                state={editor.state}
                boardIoStates={boardIoStateMap}
                displayBuffers={simState.displayBuffers}
                validationMessage={canvasValidationMessage}
                invalidPins={invalidPins}
                onMovePart={editor.movePart}
                onResizePart={editor.resizePart}
                onSelect={(id, add) => {
                  editor.select(id, add);
                  // Every component opens its own floating property window
                  // when clicked (chips get the rich inspector, other parts
                  // their properties). Clicking empty canvas (id null) or a
                  // wire opens nothing.
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
                // Anchored control toolbar — only for MCU parts (a chip's
                // intrinsic verbs live next to it on the canvas).
                selectedPartOverlay={(part) => {
                  if (!mcuBoardForPart(part, selectedBoard)) return null;
                  return renderChipControls(part.id === 'mcu', 'toolbar');
                }}
              />
            </div>
          </div>

          {/* Bottom panel — tabbed: output / serial / registers / trace / memory */}
          {showBottomPanel && (
            <div className="editor-bottom-pane">
              <div className="bottom-tabs">
                {(['output', 'serial', 'registers', 'trace', 'memory'] as BottomTab[]).map((tab) => (
                  <button
                    key={tab}
                    className={`bottom-tab ${bottomTab === tab ? 'active' : ''} ${
                      !simActive && tab !== 'output' && tab !== 'serial' ? 'disabled' : ''
                    }`}
                    onClick={() => setBottomTab(tab)}
                    disabled={!simActive && tab !== 'output' && tab !== 'serial'}
                  >
                    {tab === 'output' ? 'Output' :
                     tab === 'serial' ? 'Serial' :
                     tab === 'registers' ? 'Registers' :
                     tab === 'trace' ? 'Trace' : 'Memory'}
                  </button>
                ))}
                <button
                  className="bottom-tab bottom-tab-close"
                  onClick={() => setShowBottomPanel(false)}
                  title="Hide panel"
                >
                  &times;
                </button>
              </div>
              <div className="bottom-content">
                {bottomContent()}
              </div>
            </div>
          )}
        </div>

        {/* Property panel (right sidebar) */}
        {showRightSidebar && (
          <div className="editor-sidebar-right">
            <PropertyPanel
              parts={selectedParts}
              onUpdateAttrs={editor.updateAttrs}
              onDelete={editor.deleteSelected}
              onRotate={editor.rotatePart}
              onResize={editor.resizePart}
            />
          </div>
        )}

        {/* Collapsed right sidebar tab */}
        {!showRightSidebar && (
          <button
            className="sidebar-toggle sidebar-toggle-right"
            onClick={() => setShowRightSidebar(true)}
            title="Show properties"
          >
            <ChevronLeftIcon size={12} />
          </button>
        )}
      </div>

      {/* ===== Loading overlay (on top of UI, not replacing it) ===== */}
      {loading && (
        <div className="loading-overlay">
          <div className="spinner" />
          <span>{compiling ? 'Compiling...' : 'Loading simulator engine...'}</span>
        </div>
      )}
    </div>
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
            onChange={(key, value) => editor.updateAttrs(partId, { [key]: value })}
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
