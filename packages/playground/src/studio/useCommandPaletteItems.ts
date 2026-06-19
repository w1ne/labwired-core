import { useMemo } from 'react';
import { COMPONENT_REGISTRY } from '@labwired/ui';
import type { CommandItem } from './CommandPalette';
import type { BoardConfig } from '../bundled-configs';
import { STARTER_LABS } from './ChipRow';
import { getComponentIcon } from './componentIcons';
import type { PaletteCategory } from './PaletteDrawer';

// Mirror of App.tsx's PALETTE_CATEGORY so icon lookup uses the same category for fallbacks.
export const COMPONENT_CATEGORIES: Record<string, PaletteCategory> = {
  adxl345: 'i2c', bme280: 'i2c', mpu6050: 'i2c', vl53l1x: 'i2c', 'oled-ssd1306': 'i2c', lcd1602: 'i2c',
  ili9341: 'spi', max31855: 'spi',
  'neo6m-gps': 'uart',
  'bg770a-cellular': 'uart',
  'iolink-master': 'uart',
  'ssd1680_tricolor_290': 'spi',
  'sn74hc165': 'spi',
  'pcd8544': 'spi',
  'ntc-thermistor': 'analog', potentiometer: 'analog', ldr: 'analog',
  led: 'gpio', button: 'gpio', 'rgb-led': 'gpio', buzzer: 'gpio',
  'seven-segment': 'gpio', 'led-matrix': 'gpio', neopixel: 'gpio',
  servo: 'gpio', 'motor-driver-l293d': 'gpio',
  'pir-sensor': 'gpio', keypad: 'gpio',
  'slide-switch': 'gpio', 'dip-switch': 'gpio', 'rotary-encoder': 'gpio',
  dht22: 'misc', ultrasonic: 'misc', resistor: 'misc',
  capacitor: 'misc', diode: 'misc', transistor: 'misc',
  'shift-register-74hc595': 'misc',
  'logic-analyzer': 'tools',
};

export interface CommandPaletteContext {
  boards: BoardConfig[];
  onLoadBoard: (board: BoardConfig) => void;
  onPickLab: (labId: string) => void;
  onAddComponent: (type: string) => void;
  onRun: () => void;
  onShare: () => void;
  onReset: () => void;
  /// Drop a new MCU into the multi-chip session. Pass a board to
  /// pre-select it; otherwise the default (nRF52840) is used.
  onAddMcu?: (board?: BoardConfig) => void;
}

export function useCommandPaletteItems(ctx: CommandPaletteContext): CommandItem[] {
  return useMemo(() => {
    const items: CommandItem[] = [];

    for (const [type, def] of COMPONENT_REGISTRY.entries()) {
      if (type === 'mcu' || type.startsWith('boards/')) continue;
      items.push({
        id: `comp:${type}`,
        bucket: 'Components',
        label: def?.label ?? type,
        hint: 'drop on canvas',
        icon: getComponentIcon(type, COMPONENT_CATEGORIES[type] ?? 'misc'),
        action: () => ctx.onAddComponent(type),
      });
    }

    for (const board of ctx.boards) {
      // Selecting a board from the palette LOADS it into the workspace (switches
      // the canvas to that board's starter diagram) — the same path the header
      // breadcrumb picker uses. (The multi-chip "add a second MCU alongside"
      // flow is a separate, not-yet-shipped feature; routing plain board
      // selection through it silently no-ops, so the canvas never switches.)
      items.push({
        id: `board:${board.boardId}`,
        bucket: 'Boards',
        label: board.name,
        hint: board.arch,
        icon: getComponentIcon(board.mcuComponentType ?? 'mcu', 'misc'),
        action: () => ctx.onLoadBoard(board),
      });
    }

    for (const lab of STARTER_LABS) {
      items.push({
        id: `lab:${lab.id}`,
        bucket: 'Examples',
        label: lab.name,
        hint: lab.locked ? lab.comingIn : 'open',
        action: () => ctx.onPickLab(lab.id),
      });
    }

    items.push(
      { id: 'act:run', bucket: 'Actions', label: 'Run simulation', hint: 'Space', action: ctx.onRun },
      { id: 'act:reset', bucket: 'Actions', label: 'Reset simulation', action: ctx.onReset },
      { id: 'act:share', bucket: 'Actions', label: 'Share project', action: ctx.onShare },
    );


    return items;
  }, [ctx]);
}
