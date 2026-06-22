import factsRaw from './catalog-facts.json';

export interface CatalogFacts {
  schema_version: number;
  /** Every device_type LabWired can wire/simulate (broad validity set). */
  device_types: string[];
  /** External peripherals a proto.cat block is expected to map (coverage set). */
  peripheral_device_types: string[];
  /**
   * Best-effort chip_family proxy: families with a bundled playground board or a
   * pin-map, minus board-variant aliases. The authoritative chip_family set
   * lives in the Rust core; treat this as a strong hint, not a closed universe.
   */
  chips: string[];
}

/** Schema version the TS side was built against. */
export const CATALOG_FACTS_SCHEMA = 1;

function readFacts(raw: unknown): CatalogFacts {
  const f = raw as CatalogFacts;
  // device_types / chips are forward-compatible primitive arrays, so a newer or
  // older schema is still readable. Do NOT throw on a version mismatch at import
  // — that would crash a consumer's whole build just for reading a string list
  // (roast #7). Only a genuinely malformed file (missing the arrays) is fatal.
  if (!Array.isArray(f.device_types) || !Array.isArray(f.chips)) {
    throw new Error('catalog-facts.json is malformed: missing device_types/chips arrays');
  }
  return f;
}

const FACTS = readFacts(factsRaw);

/** True when the facts file matches the schema version these types expect. */
export const schemaMatches = FACTS.schema_version === CATALOG_FACTS_SCHEMA;

/**
 * Opt-in hard check. Consumers that want to fail on a schema mismatch (rather
 * than tolerate it) call this explicitly — importing the package never crashes
 * on its own.
 */
export function assertSchemaCompatible(): void {
  if (!schemaMatches) {
    throw new Error(
      `catalog facts schema mismatch: file=${FACTS.schema_version}, ts=${CATALOG_FACTS_SCHEMA}. ` +
        `Bump @labwired/catalog to a version built for schema ${FACTS.schema_version}.`,
    );
  }
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
