import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { CATALOG } from '../../board-config/src/catalog';
import { PIN_MAPS } from '../../board-config/src/pin-mapping';
import { PLAYGROUND_BOARD_CATALOG } from '../../board-config/src/boards';
import manifest from '../../ui/src/peripherals/manifest.json' with { type: 'json' };

const SCHEMA_VERSION = 1;
const OUT = fileURLToPath(new URL('../src/catalog-facts.json', import.meta.url));

// CATALOG deviceClasses that are bus-attached external devices — the kind a
// proto.cat block composes. MCUs are chips, and passive/board_io/tool parts are
// not composable peripherals, so they are excluded from the coverage set.
const EXTERNAL_DEVICE_CLASSES = new Set(['i2c_device', 'spi_device', 'uart_device']);

// PIN_MAPS keys that are board-specific aliases, not distinct chip families —
// they reuse a base family's pin map and never appear as a board `board` value.
// Excluded so `chips` stays a clean family list. The authoritative chip_family
// set lives in the Rust core; this TS-side list is a best-effort proxy built
// from real bundled boards + the pin-map families.
const CHIP_ALIASES = new Set(['esp32-s3-zero', 'nrf52840-onboarding']);

function sortedUnique(values: string[]): string[] {
  return [...new Set(values)].sort();
}

function buildChips(): string[] {
  const fromBoards = PLAYGROUND_BOARD_CATALOG.map((b) => b.board);
  const fromPinMaps = Object.keys(PIN_MAPS).filter((k) => !CHIP_ALIASES.has(k));
  return sortedUnique([...fromBoards, ...fromPinMaps]);
}

function build(): string {
  const catalogParts = Object.values(CATALOG);
  const catalogTypes = catalogParts.map((p) => p.type);
  const externalCatalogTypes = catalogParts
    .filter((p) => EXTERNAL_DEVICE_CLASSES.has(p.deviceClass))
    .map((p) => p.type);
  const manifestTypes = (manifest.peripherals as { device_type: string }[]).map(
    (p) => p.device_type,
  );
  const facts = {
    schema_version: SCHEMA_VERSION,
    // Broad validity set: every device_type LabWired can wire/simulate.
    device_types: sortedUnique([...catalogTypes, ...manifestTypes]),
    // Coverage set: external peripherals a proto.cat block is expected to map
    // (all kit-registered peripherals + bus-attached catalog devices).
    peripheral_device_types: sortedUnique([...manifestTypes, ...externalCatalogTypes]),
    // Best-effort chip_family proxy: real bundled-board families + pin-map
    // families, minus board-variant aliases. See CHIP_ALIASES above.
    chips: buildChips(),
  };
  return JSON.stringify(facts, null, 2) + '\n';
}

const generated = build();
if (process.argv.includes('--check')) {
  const current = readFileSync(OUT, 'utf8');
  if (current !== generated) {
    console.error(
      'packages/catalog/src/catalog-facts.json is stale. Run: npm --prefix packages/catalog run generate:facts',
    );
    process.exit(1);
  }
} else {
  writeFileSync(OUT, generated);
  console.error(`wrote ${OUT}`);
}
