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
  lab: LabRef | null;
}

interface Manifest {
  schema_version: number;
  peripherals: KitMetadata[];
}

const MANIFEST = manifestRaw as Manifest;

/** Schema version the TS side was built against. Generator must match. */
export const PERIPHERAL_MANIFEST_SCHEMA = 1;

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

/** Lookup by the boardId of the kit's associated starter lab. */
export function findKitByBoardId(boardId: string): KitMetadata | undefined {
  return PERIPHERAL_KITS.find((k) => k.lab?.board_id === boardId);
}

/** All kits that ship a starter lab (every kit, currently — but the type
 *  allows lab-less kits, so we filter explicitly).
 */
export function kitsWithLabs(): KitMetadata[] {
  return PERIPHERAL_KITS.filter((k): k is KitMetadata & { lab: LabRef } => k.lab !== null);
}
