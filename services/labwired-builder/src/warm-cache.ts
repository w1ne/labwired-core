// Build-time framework baker. Runs during `docker build` (with network access)
// and compiles a trivial sketch for EVERY catalog board, so the running
// container can compile OFFLINE — it is deployed on an egress-denied network.
//
// It derives entirely from the board catalog (src/boards.ts) via the SAME
// generatePlatformioIni() the runtime uses, so the baked toolchain always
// matches exactly what /compile will ask for. There is no second board list to
// keep in sync: add a board to src/boards.ts and it is baked here automatically.
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { bakeTargets } from './compile.js';

const pio = process.env.PIO_BIN ?? 'pio';
// Every distinct platformio.ini the runtime can ask for (boards ∪ chip families),
// deduped — derived entirely from board-catalog.json. No second list.
const targets = bakeTargets();
const failures: string[] = [];

for (const { label, ini, isArduino } of targets) {
  const dir = mkdtempSync(join(tmpdir(), 'warm-'));
  try {
    mkdirSync(join(dir, 'src'));
    writeFileSync(join(dir, 'platformio.ini'), ini);
    // Arduino boards need a setup()/loop() sketch (Arduino.h gives the right C++
    // linkage); stm32cube / bare-metal boards take a plain main().
    if (isArduino) {
      writeFileSync(join(dir, 'src', 'main.cpp'), '#include <Arduino.h>\nvoid setup(void){}\nvoid loop(void){}\n');
    } else {
      writeFileSync(join(dir, 'src', 'main.c'), 'int main(void){while(1){}}\n');
    }
    process.stdout.write(`=== warming ${label} ===\n`);
    execFileSync(pio, ['run', '-d', dir, '-e', 'sim'], { stdio: 'inherit' });
  } catch (err) {
    failures.push(`${label}: ${err instanceof Error ? err.message : String(err)}`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (failures.length) {
  process.stderr.write(`\nframework bake FAILED for ${failures.length} target(s):\n${failures.join('\n')}\n`);
  process.exit(1);
}
process.stdout.write(`\nbaked ${targets.length} target(s) successfully\n`);
