import factsRaw from './catalog-facts.json';

export interface CatalogFacts {
  schema_version: number;
  /** Every device_type LabWired can wire/simulate (broad validity set). */
  device_types: string[];
  /** External peripherals a proto.cat block is expected to map (coverage set). */
  peripheral_device_types: string[];
  /** Known chip families (== compile-service chip_family). */
  chips: string[];
}

/** Schema version the TS side was built against. Generator must match. */
export const CATALOG_FACTS_SCHEMA = 1;

const FACTS = factsRaw as CatalogFacts;

if (FACTS.schema_version !== CATALOG_FACTS_SCHEMA) {
  throw new Error(
    `catalog facts schema mismatch: file=${FACTS.schema_version}, ts=${CATALOG_FACTS_SCHEMA}. ` +
      `Re-run \`npm --prefix packages/catalog run generate:facts\`.`,
  );
}

export const CATALOG_FACTS: CatalogFacts = FACTS;
export const DEVICE_TYPES: readonly string[] = FACTS.device_types;
export const PERIPHERAL_DEVICE_TYPES: readonly string[] = FACTS.peripheral_device_types;
export const CHIPS: readonly string[] = FACTS.chips;

const DEVICE_TYPE_SET = new Set(FACTS.device_types);
const CHIP_SET = new Set(FACTS.chips);

/** True if `deviceType` is a real LabWired device_type (any class). */
export function isKnownDeviceType(deviceType: string): boolean {
  return DEVICE_TYPE_SET.has(deviceType);
}

/** True if `chip` is a real LabWired chip_family. */
export function isKnownChip(chip: string): boolean {
  return CHIP_SET.has(chip);
}
