// Synthesize a LabWired editor board (Diagram/EditorState) from a device's system
// manifest, so a read-only consumer (e.g. proto.cat's device page) can render the
// shared EditorCanvas — the same board view the playground uses — and light the
// e-paper up live via displayBuffers, instead of a bespoke canvas.
//
// EditorCanvas can only draw parts it has a ComponentDef for, so `canRenderInEditor`
// gates this: when a device uses a part the registry doesn't know, the caller falls
// back to whatever generic renderer it has (a box per part).

import type { Diagram, EditorState, Part, Wire } from './types';

// ESP32 SPI bus → MCU GPIO pin ids (SCK/MOSI are implied by `connection`, not
// structural). Pin ids match the esp32 board component (GPIO<n>).
const SPI_BUS: Record<string, { sck: string; mosi: string }> = {
  spi2: { sck: 'GPIO14', mosi: 'GPIO13' }, // HSPI
  spi3: { sck: 'GPIO18', mosi: 'GPIO23' }, // VSPI (the e-reader)
  hspi: { sck: 'GPIO14', mosi: 'GPIO13' },
  vspi: { sck: 'GPIO18', mosi: 'GPIO23' },
};

// COMPONENT_REGISTRY keys that exist in the editor.
const MCU_TYPES: Record<string, string> = {
  esp32: 'esp32',
  'esp32-wroom': 'esp32',
  esp32s3: 'esp32-s3-zero',
  esp32c3: 'esp32-c3-supermini',
};
const PANEL_REGISTRY_TYPES = new Set(['ssd1680_tricolor_290', 'uc8151d_tricolor_290']);

interface ExternalDevice {
  id: string;
  type: string;
  connection: string;
  config: Record<string, string>;
}
interface ParsedManifest {
  chip: string;
  externalDevices: ExternalDevice[];
}

/** Hand-rolled YAML-subset parser for the LabWired system manifest. */
function parseManifest(manifest: string): ParsedManifest {
  const result: ParsedManifest = { chip: 'esp32', externalDevices: [] };
  const lines = manifest.split('\n');
  let inExternalDevices = false;
  let inConfig = false;
  let current: ExternalDevice | null = null;
  const stripComment = (s: string) => s.replace(/\s+#.*$/, '').trim();
  const unquote = (s: string) => s.replace(/^["']|["']$/g, '').trim();
  for (const raw of lines) {
    const line = raw.replace(/\t/g, '  ');
    const trimmed = stripComment(line);
    if (!trimmed) continue;
    const chipMatch = trimmed.match(/^chip\s*:\s*(.+)$/);
    if (chipMatch && !inExternalDevices) {
      result.chip = unquote(chipMatch[1]);
      continue;
    }
    if (/^external_devices\s*:/.test(trimmed)) {
      inExternalDevices = true;
      continue;
    }
    if (inExternalDevices && /^\S/.test(line) && !trimmed.startsWith('-')) {
      if (current) result.externalDevices.push(current);
      current = null;
      inExternalDevices = false;
      inConfig = false;
      continue;
    }
    if (!inExternalDevices) continue;
    const isNewEntry = trimmed.startsWith('-');
    const entryBody = isNewEntry ? trimmed.replace(/^-\s*/, '') : trimmed;
    if (isNewEntry) {
      if (current) result.externalDevices.push(current);
      current = { id: '', type: '', connection: '', config: {} };
      inConfig = false;
    }
    if (!current) continue;
    if (/^config\s*:/.test(entryBody)) {
      inConfig = true;
      continue;
    }
    const kv = entryBody.match(/^([a-zA-Z0-9_]+)\s*:\s*(.*)$/);
    if (!kv) continue;
    const key = kv[1];
    const value = unquote(kv[2]);
    if (key === 'id') {
      current.id = value;
      inConfig = false;
    } else if (key === 'type') {
      current.type = value;
      inConfig = false;
    } else if (key === 'connection') {
      current.connection = value;
      inConfig = false;
    } else if (inConfig && value) {
      current.config[key] = value;
    }
  }
  if (current) result.externalDevices.push(current);
  return result;
}

function chipKey(chip: string): string {
  return (chip.split('/').pop()?.replace(/\.ya?ml$/i, '') ?? chip).toLowerCase();
}

function mcuType(chip: string): string | null {
  return MCU_TYPES[chipKey(chip)] ?? null;
}

function panelType(devType: string): string | null {
  const t = devType.toLowerCase();
  if (t.includes('uc8151')) return 'uc8151d_tricolor_290';
  if (t.includes('ssd1680') || t.includes('epaper') || t.includes('e-paper') || t.includes('e_paper'))
    return 'ssd1680_tricolor_290';
  return null;
}

// I2C sensors the editor can render (manifest device type → registry component).
const I2C_SENSOR_TYPES: Record<string, string> = {
  mlx90640: 'mlx90640',
  bme280: 'bme280',
  adxl345: 'adxl345',
  mpu6050: 'mpu6050',
};

/** Map an I2C sensor manifest type to a renderable component type. */
function sensorType(devType: string): string | null {
  const t = devType.toLowerCase();
  if (I2C_SENSOR_TYPES[t]) return I2C_SENSOR_TYPES[t];
  if (t.includes('mlx90640')) return 'mlx90640';
  return null;
}

/** Map an IO-Link / M12 field connector manifest type to a component type. */
function connectorType(devType: string): string | null {
  const t = devType.toLowerCase();
  if (t.includes('m12') || t.includes('iolink') || t.includes('io-link') || t.includes('io_link'))
    return 'm12-iolink';
  return null;
}

/** Any external-device type the editor has a ComponentDef for. */
function renderableType(devType: string): string | null {
  return panelType(devType) ?? sensorType(devType) ?? connectorType(devType);
}

/** Normalize "5" | "GPIO5" | "IO5" → "GPIO5" (esp32 component pin ids). */
function gpio(raw: string): string {
  return `GPIO${String(raw).replace(/[^0-9]/g, '')}`;
}

/**
 * True only when EVERY part of the device maps to a ComponentDef (a known MCU +
 * only renderable peripherals). Otherwise the caller should use its fallback
 * renderer so nothing goes unrendered.
 */
export function canRenderInEditor(systemManifest: string | null): boolean {
  if (!systemManifest) return false;
  let parsed: ParsedManifest;
  try {
    parsed = parseManifest(systemManifest);
  } catch {
    return false;
  }
  if (!mcuType(parsed.chip)) return false;
  if (parsed.externalDevices.length === 0) return false;
  // Render only when every external device maps to a ComponentDef — tri-color
  // panels, known I2C sensors, or an IO-Link/M12 connector. Anything else falls
  // back to the caller's generic renderer so nothing goes undrawn.
  return parsed.externalDevices.every((d) => renderableType(d.type) !== null);
}

/**
 * Build a read-only EditorState from the manifest. The panel Part's id is set to
 * the device id so `displayBuffers[id]` routes the live framebuffer into the
 * canvas. Returns null if the device can't be rendered in the editor.
 */
export function manifestToEditorState(systemManifest: string | null): EditorState | null {
  if (!canRenderInEditor(systemManifest)) return null;
  const parsed = parseManifest(systemManifest as string);
  const mcu = mcuType(parsed.chip) as string;

  const parts: Part[] = [{ id: 'mcu', type: mcu, x: 60, y: 40, rotate: 0, attrs: {} }];
  const wires: Wire[] = [];
  const W = (c: Wire) => wires.push(c);

  parsed.externalDevices.forEach((dev, i) => {
    const partId = dev.id || `dev${i}`;
    const panel = panelType(dev.type);
    const sensor = sensorType(dev.type);
    const connector = connectorType(dev.type);
    const type = (panel ?? sensor ?? connector) as string;
    parts.push({ id: partId, type, x: 460, y: 120 + i * 200, rotate: 0, attrs: {} });

    // Power rails — panels and I2C sensors carry VCC/GND pads; the M12 field
    // connector instead exposes L+/L- (bus power) and is wired below.
    if (panel || sensor) {
      W({ from: { part: 'mcu', pin: '3V3' }, to: { part: partId, pin: 'VCC' }, color: '#FF6B6B' });
      W({ from: { part: 'mcu', pin: 'GND' }, to: { part: partId, pin: 'GND' }, color: '#888888' });
    }

    if (panel) {
      const bus = SPI_BUS[dev.connection.toLowerCase()];
      if (bus && PANEL_REGISTRY_TYPES.has(type)) {
        const { cs_pin, dc_pin, rst_pin, reset_pin, busy_pin } = dev.config;
        W({ from: { part: 'mcu', pin: bus.sck }, to: { part: partId, pin: 'CLK' }, color: '#5BD8FF' });
        W({ from: { part: 'mcu', pin: bus.mosi }, to: { part: partId, pin: 'DIN' }, color: '#B07BFF' });
        if (cs_pin) W({ from: { part: 'mcu', pin: gpio(cs_pin) }, to: { part: partId, pin: 'CS' }, color: '#3DD68C' });
        if (dc_pin) W({ from: { part: 'mcu', pin: gpio(dc_pin) }, to: { part: partId, pin: 'DC' }, color: '#5B9DFF' });
        const rst = rst_pin ?? reset_pin;
        if (rst) W({ from: { part: 'mcu', pin: gpio(rst) }, to: { part: partId, pin: 'RST' }, color: '#F5B642' });
        if (busy_pin) W({ from: { part: 'mcu', pin: gpio(busy_pin) }, to: { part: partId, pin: 'BUSY' }, color: '#FFE680' });
      }
    } else if (sensor) {
      // I2C bus: SDA/SCL pins come from the device config.
      const { scl_pin, sda_pin } = dev.config;
      if (scl_pin) W({ from: { part: 'mcu', pin: gpio(scl_pin) }, to: { part: partId, pin: 'SCL' }, color: '#5BD8FF' });
      if (sda_pin) W({ from: { part: 'mcu', pin: gpio(sda_pin) }, to: { part: partId, pin: 'SDA' }, color: '#B07BFF' });
    } else if (connector) {
      // IO-Link field connector: C/Q to the MCU UART line (via a PHY), L- to GND.
      const { cq_pin, tx_pin } = dev.config;
      const cq = cq_pin ?? tx_pin;
      if (cq) W({ from: { part: 'mcu', pin: gpio(cq) }, to: { part: partId, pin: 'CQ' }, color: '#37D67A' });
      W({ from: { part: 'mcu', pin: 'GND' }, to: { part: partId, pin: 'L-' }, color: '#888888' });
    }
  });

  const diagram: Diagram = { version: 1, board: chipKey(parsed.chip), parts, wires };
  return { diagram, selectedIds: new Set(), wireInProgress: null, undoStack: [], redoStack: [] };
}
