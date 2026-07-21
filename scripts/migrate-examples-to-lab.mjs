#!/usr/bin/env node
// Migrate LabWired example projects to the canonical run file:
//   <project>/.labwired/lab.yaml
//
// For every example dir under core/examples/** that contains a system.yaml,
// create <dir>/.labwired/lab.yaml using the EXISTING labwired test-script schema.
// Paths in inputs.* are resolved relative to the lab.yaml's own directory
// (i.e. relative to the .labwired/ subfolder), so any firmware/system path
// copied from a script that lived in <dir> is rewritten accordingly.
//
// Deterministic + idempotent: re-running skips dirs that already have lab.yaml.

import fs from "node:fs";
import path from "node:path";

const ROOT = process.cwd();
// Works from the superproject root (core/examples) or a core checkout root (examples).
// Override with LABWIRED_EXAMPLES_DIR when neither default applies.
const EXAMPLES =
  process.env.LABWIRED_EXAMPLES_DIR ||
  (fs.existsSync(path.join(ROOT, "core", "examples"))
    ? path.join(ROOT, "core", "examples")
    : path.join(ROOT, "examples"));

// --- enumerate example dirs (those directly containing a system.yaml) --------
function findSystemDirs(base) {
  const out = [];
  const walk = (dir) => {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    // record this dir if it holds a system.yaml
    if (entries.some((e) => e.isFile() && e.name === "system.yaml")) {
      out.push(dir);
    }
    for (const e of entries) {
      if (!e.isDirectory()) continue;
      const name = e.name;
      // never descend into worktree / build / fixture noise
      if (
        name === "node_modules" ||
        name === ".wt" ||
        name === "wt-riscv-jit-enable" ||
        name === ".worktrees" ||
        name === "target" ||
        name === "ci" // core/examples/ci dummies have no system.yaml anyway
      ) {
        continue;
      }
      walk(path.join(dir, name));
    }
  };
  walk(base);
  return out.sort();
}

// --- minimal detection: does a yaml file look like a test-script? ------------
function looksLikeTestScript(text) {
  return /(^|\n)\s*schema_version\s*:/.test(text) &&
    /(^|\n)\s*assertions\s*:/.test(text);
}

// --- pick the best candidate script in a dir ---------------------------------
function pickScript(dir) {
  const files = fs
    .readdirSync(dir, { withFileTypes: true })
    .filter((e) => e.isFile())
    .map((e) => e.name)
    .filter((n) => /\.ya?ml$/i.test(n) && n !== "system.yaml");

  const candidates = files
    .filter((n) => {
      try {
        return looksLikeTestScript(fs.readFileSync(path.join(dir, n), "utf8"));
      } catch {
        return false;
      }
    })
    .sort(canonicalFirst);

  if (candidates.length === 0) return null;

  const bySmoke = candidates.filter((n) => /smoke/i.test(n));
  if (bySmoke.length) return bySmoke.sort(canonicalFirst)[0];
  const byTest = candidates.filter((n) => /test/i.test(n));
  if (byTest.length) return byTest.sort(canonicalFirst)[0];
  return candidates[0];
}

// Prefer the bare/canonical script name over suffixed variants:
// `test.yaml` beats `test-fresh.yaml`, `uds-smoke.yaml` beats `uds-reset-smoke.yaml`.
// Shortest name wins (bare has no `-<variant>` suffix), then alphabetical.
function canonicalFirst(a, b) {
  return a.length - b.length || a.localeCompare(b);
}

// --- rewrite an inputs.* value for the new .labwired/ location ---------------
// oldValue was relative to `dir` (the script lived there); the new file lives in
// `dir/.labwired`, so recompute the relative path to the same resolved target.
function rewriteValue(dir, oldValue) {
  const resolved = path.resolve(dir, oldValue);
  const newDir = path.join(dir, ".labwired");
  let rel = path.relative(newDir, resolved);
  if (!rel.startsWith(".") && !path.isAbsolute(rel)) rel = "./" + rel;
  return rel;
}

// --- strip surrounding quotes from a scalar ----------------------------------
function unquote(s) {
  s = s.trim();
  if (
    (s.startsWith('"') && s.endsWith('"')) ||
    (s.startsWith("'") && s.endsWith("'"))
  ) {
    return s.slice(1, -1);
  }
  return s;
}

// --- transform a whole script's text, rewriting only inputs.{firmware,system}-
// Preserves comments, limits, assertions verbatim.
function transformScript(dir, text) {
  const lines = text.split("\n");
  let inInputs = false;
  let inputsIndent = 0;

  const isKeyLine = (line) => /^(\s*)([A-Za-z0-9_.-]+)\s*:/.exec(line);

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.trim() === "" || /^\s*#/.test(line)) continue;

    const m = isKeyLine(line);
    if (!m) continue;
    const indent = m[1].length;
    const key = m[2];

    if (!inInputs) {
      if (indent === 0 && key === "inputs") {
        inInputs = true;
        inputsIndent = indent;
      }
      continue;
    }

    // inside inputs: a key at the same-or-lower indent than `inputs:` ends it
    if (indent <= inputsIndent) {
      inInputs = false;
      // re-evaluate this line as a possible new top-level inputs (won't be)
      if (indent === 0 && key === "inputs") {
        inInputs = true;
        inputsIndent = indent;
      }
      continue;
    }

    if (key === "firmware" || key === "system") {
      const colon = line.indexOf(":");
      const rawVal = line.slice(colon + 1);
      const oldValue = unquote(rawVal);
      if (oldValue === "") continue; // nested block, leave alone
      const newValue = rewriteValue(dir, oldValue);
      lines[i] = `${" ".repeat(indent)}${key}: "${newValue}"`;
    }
  }
  return lines.join("\n");
}

const MINIMAL = `schema_version: "1.0"
inputs:
  system: "../system.yaml"
limits:
  max_steps: 100000
assertions:
  - expected_stop_reason: max_steps
`;

// --- main --------------------------------------------------------------------
function main() {
  const dirs = findSystemDirs(EXAMPLES);
  let migrated = 0;
  let created = 0;
  let skipped = 0;

  for (const dir of dirs) {
    const labDir = path.join(dir, ".labwired");
    const labFile = path.join(labDir, "lab.yaml");
    const rel = path.relative(ROOT, dir);

    if (fs.existsSync(labFile)) {
      console.log(`SKIP ${rel} (exists)`);
      skipped++;
      continue;
    }

    const script = pickScript(dir);
    fs.mkdirSync(labDir, { recursive: true });

    if (script) {
      const text = fs.readFileSync(path.join(dir, script), "utf8");
      const out = transformScript(dir, text);
      fs.writeFileSync(labFile, out);
      console.log(`MIGRATED ${rel} from ${script}`);
      migrated++;
    } else {
      fs.writeFileSync(labFile, MINIMAL);
      console.log(`CREATED ${rel} (minimal)`);
      created++;
    }
  }

  console.log(
    `\nDONE  migrated=${migrated} created=${created} skipped=${skipped} total=${dirs.length}`
  );
}

main();
