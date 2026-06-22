import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { CATALOG } from '../../board-config/src/catalog';
import { PIN_MAPS } from '../../board-config/src/pin-mapping';
import manifest from '../../ui/src/peripherals/manifest.json' with { type: 'json' };

const SCHEMA_VERSION = 1;
const OUT = fileURLToPath(new URL('../src/catalog-facts.json', import.meta.url));

// CATALOG deviceClasses that are bus-attached external devices — the kind a
// proto.cat block composes. MCUs are chips, and passive/board_io/tool parts are
// not composable peripherals, so they are excluded from the coverage set.
const EXTERNAL_DEVICE_CLASSES = new Set(['i2c_device', 'spi_device', 'uart_device']);

function sortedUnique(values: string[]): string[] {
  return [...new Set(values)].sort();
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
    chips: sortedUnique(Object.keys(PIN_MAPS)),
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
