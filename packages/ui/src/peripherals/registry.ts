// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

/**
 * TypeScript view of the unified peripheral contract.
 *
 * `manifest.json` is the offline-generated mirror of the Rust
 * `peripherals::kit::registry::KITS` slice (see
 * `core/crates/cli/src/bin/gen_peripherals_manifest.rs`). The Rust side is
 * the source of truth — CI is the product, the browser is the demo layer —
 * so this module never invents metadata; it only re-shapes the manifest
 * into the small helpers the playground / library / palette consume.
 *
 * Adding a peripheral never starts here. You implement `PeripheralKit` in
 * core, append it to `KITS`, re-run the generator, and the manifest pulls
 * everything else into line.
 */

import manifestRaw from './manifest.json';

export type Transport = 'uart' | 'i2c' | 'spi' | 'analog' | 'gpio_group';
export type Category = 'uart' | 'i2c' | 'spi' | 'analog' | 'gpio' | 'misc';
export type ConfigType = 'str' | 'int' | 'bool' | 'float';

export interface ConfigKey {
  name: string;
  ty: ConfigType;
  doc: string;
}

export interface LabRef {
  board_id: string;
  chip: string;
  example_dir: string;
  demo_elf: string;
}

export interface KitMetadata {
  device_type: string;
  label: string;
  summary: string;
  detail: string;
  transport: Transport;
  category: Category;
  config_keys: ConfigKey[];
  /**
   * Starter labs the peripheral ships with. Zero entries = no demo lab
   * yet (the kit is library-only). One entry is the common case. Multiple
   * entries cover peripherals reused across chips (e.g. an e-paper model
   * that has both STM32 and ESP32 labs).
   */
  labs: LabRef[];
}

interface Manifest {
  schema_version: number;
  peripherals: KitMetadata[];
}

const MANIFEST = manifestRaw as Manifest;

/** Schema version the TS side was built against. Generator must match. */
export const PERIPHERAL_MANIFEST_SCHEMA = 2;

if (MANIFEST.schema_version !== PERIPHERAL_MANIFEST_SCHEMA) {
  throw new Error(
    `peripheral manifest schema mismatch: file=${MANIFEST.schema_version}, ts=${PERIPHERAL_MANIFEST_SCHEMA}. ` +
      `Re-run \`cargo run -p labwired-cli --bin gen-peripherals-manifest -- --out packages/ui/src/peripherals/manifest.json\`.`,
  );
}

/** Every peripheral registered through `PeripheralKit`. */
export const PERIPHERAL_KITS: readonly KitMetadata[] = MANIFEST.peripherals;

/** Lookup by `device_type` string (same string as `type:` in system.yaml). */
export function findKit(deviceType: string): KitMetadata | undefined {
  return PERIPHERAL_KITS.find((k) => k.device_type === deviceType);
}

/** Lookup by the boardId of any of the kit's associated starter labs. */
export function findKitByBoardId(boardId: string): KitMetadata | undefined {
  return PERIPHERAL_KITS.find((k) => k.labs.some((l) => l.board_id === boardId));
}

/** All (kit, lab) pairs across every registered kit. A kit with two labs
 * yields two pairs; a kit with zero labs yields none. Most surface gates
 * want to assert against the pair, not the kit.
 */
export function kitLabs(): { kit: KitMetadata; lab: LabRef }[] {
  return PERIPHERAL_KITS.flatMap((kit) => kit.labs.map((lab) => ({ kit, lab })));
}

/** All kits that ship at least one starter lab. */
export function kitsWithLabs(): KitMetadata[] {
  return PERIPHERAL_KITS.filter((k) => k.labs.length > 0);
}
