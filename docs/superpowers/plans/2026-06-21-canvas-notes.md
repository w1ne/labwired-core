# Canvas Notes + Lab Descriptions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a placeable, editable text **note** to the playground canvas, and ship a short description note on every example lab so users know what they're looking at.

**Architecture:** A note is a normal `Part` with `type: 'note'`, category `tool` (the same inert category as Logic Analyzer) — no pins, no `boardIoKind`, no wires, so it's invisible to `diagramToConfig`, validation, and the simulator. It serializes with the diagram for free. Lab descriptions are seeded as a note in `makeStarterDiagram` from a `LAB_NOTES` map.

**Tech Stack:** React + TypeScript, SVG canvas (`<foreignObject>` for wrapped/editable text), Vitest + Testing Library.

## Global Constraints

- Work in worktree `~/projects/labwired/.worktrees/canvas-notes`, branch `feat/canvas-notes` (off latest `origin/main`).
- No Claude/AI references in commit messages.
- Commit author identity is the machine default (w1ne noreply) — do not override.
- Run unit tests from `packages/playground` with `npx vitest run <file>` and from `packages/ui` likewise.
- A note must NEVER produce a `board_io` binding or a validation message.
- Only non-hidden `kind: 'lab'` configs get a seeded note. Bare boards and `hidden: true` configs do not.
- The lab blurb text in `LAB_NOTES` is **user-review-gated**: Task 4 Step 1 presents the full draft to the user and applies their edits BEFORE the map is written/committed.

---

### Task 1: Note component + registration

**Files:**
- Create: `packages/ui/src/editor/components/note.tsx`
- Modify: `packages/ui/src/editor/components/index.ts` (import + add to registry array)
- Test: `packages/ui/src/editor/components/note.test.tsx`

**Interfaces:**
- Consumes: `ComponentDef`, `AttrFieldDef` from `../types`; `getComponentsByCategory`, `COMPONENT_REGISTRY` from `./index`.
- Produces: `noteComponent: ComponentDef` with `type: 'note'`, `category: 'tool'`, `pins: []`, no `boardIoKind`, `defaultAttrs: { text: 'Double-click to edit' }`, and an `attrFields` entry `{ key: 'text', label: 'Text', type: 'textarea' }`. The `'textarea'` value is added to the `AttrFieldDef` type union in Step 1 of this task; the PropertyPanel **rendering** of that type is added in Task 2.

- [ ] **Step 1: Add `'textarea'` to the `AttrFieldDef` type union**

In `packages/ui/src/editor/types.ts`, change the `type` field of `AttrFieldDef`:

```ts
  type: 'text' | 'select' | 'color' | 'range' | 'textarea';
```

- [ ] **Step 2: Write the failing test**

```tsx
// packages/ui/src/editor/components/note.test.tsx
import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import { COMPONENT_REGISTRY, getComponentsByCategory } from './index';

describe('note component', () => {
  it('is registered as an inert tool with no pins or board binding', () => {
    const def = COMPONENT_REGISTRY.get('note');
    expect(def).toBeDefined();
    expect(def!.category).toBe('tool');
    expect(def!.pins).toEqual([]);
    expect(def!.boardIoKind).toBeUndefined();
  });

  it('appears under the Tools palette group', () => {
    const groups = getComponentsByCategory();
    expect(groups.tool?.some((d) => d.type === 'note')).toBe(true);
  });

  it('renders the attr text without throwing (empty and long)', () => {
    const def = COMPONENT_REGISTRY.get('note')!;
    const long = 'x'.repeat(400);
    expect(() =>
      render(<svg>{def.render({ text: '' }, { id: 'n1' })}</svg>),
    ).not.toThrow();
    expect(() =>
      render(<svg>{def.render({ text: long }, { id: 'n2' })}</svg>),
    ).not.toThrow();
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd packages/ui && npx vitest run src/editor/components/note.test.tsx`
Expected: FAIL — `COMPONENT_REGISTRY.get('note')` is `undefined`.

- [ ] **Step 4: Create the note component**

```tsx
// packages/ui/src/editor/components/note.tsx
import type { ComponentDef } from '../types';

const W = 220;
const PAD = 12;

/**
 * A free-form text annotation. Not a circuit element: no pins, no boardIoKind,
 * no wires — inert to diagramToConfig, validation, and the simulator. Text
 * lives in `attrs.text`. Rendered with a <foreignObject> so the body wraps and
 * the card grows in height (plain SVG <text> can't wrap). Inline editing is
 * handled in EditorCanvas (double-click), with a textarea fallback in the
 * PropertyPanel.
 */
export const noteComponent: ComponentDef = {
  type: 'note',
  label: 'Note',
  category: 'tool',
  width: W,
  height: 96, // nominal; real height comes from content via foreignObject
  pins: [],
  defaultAttrs: { text: 'Double-click to edit' },
  attrFields: [{ key: 'text', label: 'Text', type: 'textarea' }],
  render: (attrs, state) => {
    const text = attrs.text ?? '';
    const selected = !!state?.selected;
    return (
      <g>
        <foreignObject x={0} y={0} width={W} height={1} overflow="visible">
          <div
            // xmlns required so HTML inside SVG <foreignObject> paints in all browsers
            xmlns="http://www.w3.org/1999/xhtml"
            style={{
              width: `${W}px`,
              boxSizing: 'border-box',
              padding: `${PAD}px`,
              background: '#fff8e1',
              border: `1.5px solid ${selected ? '#F5B642' : '#e6d59a'}`,
              borderRadius: '8px',
              boxShadow: '0 1px 3px rgba(0,0,0,0.18)',
              font: "12px/1.45 -apple-system, 'Segoe UI', sans-serif",
              color: '#4a3f1e',
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
            }}
          >
            {text === '' ? ' ' : text}
          </div>
        </foreignObject>
      </g>
    );
  },
};
```

- [ ] **Step 5: Register the component**

In `packages/ui/src/editor/components/index.ts`, under the `// Tools` import group:

```ts
import { logicAnalyzerComponent } from './logic-analyzer';
import { noteComponent } from './note';
```

Then add `noteComponent` to the array that the registry/`getComponentsByCategory` is built from (the same list `logicAnalyzerComponent` is in — add it right after).

- [ ] **Step 6: Run tests to verify they pass**

Run: `cd packages/ui && npx vitest run src/editor/components/note.test.tsx`
Expected: PASS (3 tests).

- [ ] **Step 7: Commit**

```bash
git add packages/ui/src/editor/components/note.tsx \
        packages/ui/src/editor/components/note.test.tsx \
        packages/ui/src/editor/components/index.ts \
        packages/ui/src/editor/types.ts
git commit -m "feat(editor): inert note component (placeable text annotation)"
```

---

### Task 2: Textarea field in PropertyPanel

**Files:**
- Modify: `packages/ui/src/editor/PropertyPanel.tsx` (attrField switch ~lines 88-138)
- Test: `packages/ui/src/editor/PropertyPanel.note.test.tsx`

**Interfaces:**
- Consumes: `noteComponent` (Task 1), `onUpdateAttrs(partId, attrs)` prop already on PropertyPanel.
- Produces: a `<textarea>` rendered for any `attrFields` entry with `type: 'textarea'`.

- [ ] **Step 1: Write the failing test**

```tsx
// packages/ui/src/editor/PropertyPanel.note.test.tsx
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { PropertyPanel } from './PropertyPanel';
import type { Part } from './types';

const notePart: Part = { id: 'note', type: 'note', x: 0, y: 0, rotate: 0, attrs: { text: 'hi' } };

describe('PropertyPanel note text', () => {
  it('renders a textarea for the note text and commits edits', () => {
    const onUpdateAttrs = vi.fn();
    render(
      <PropertyPanel
        parts={[notePart]}
        onUpdateAttrs={onUpdateAttrs}
        onDelete={() => {}}
        onRotate={() => {}}
      />,
    );
    const ta = screen.getByLabelText('Text') as HTMLTextAreaElement;
    expect(ta.tagName).toBe('TEXTAREA');
    expect(ta.value).toBe('hi');
    fireEvent.change(ta, { target: { value: 'updated' } });
    expect(onUpdateAttrs).toHaveBeenCalledWith('note', { text: 'updated' });
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/ui && npx vitest run src/editor/PropertyPanel.note.test.tsx`
Expected: FAIL — the field renders as a single-line `<input>`, `tagName` is `INPUT`.

- [ ] **Step 3: Add the textarea branch**

In `PropertyPanel.tsx`, in the attrField conditional, add a `'textarea'` branch before the final `else` (`<input type="text">`). Insert after the `'range'` block's closing `) : `:

```tsx
              ) : field.type === 'textarea' ? (
                <textarea
                  className="panel-input"
                  rows={4}
                  aria-label={field.label}
                  value={part.attrs[field.key] || ''}
                  onChange={(e) =>
                    onUpdateAttrs(part.id, { [field.key]: e.target.value })
                  }
                />
```

(So the chain reads `select ? … : range ? … : textarea ? … : <input>`.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cd packages/ui && npx vitest run src/editor/PropertyPanel.note.test.tsx`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/ui/src/editor/PropertyPanel.tsx packages/ui/src/editor/PropertyPanel.note.test.tsx
git commit -m "feat(editor): textarea attribute field for multiline note text"
```

---

### Task 3: Inline note editing on the canvas

**Files:**
- Modify: `packages/ui/src/editor/EditorCanvas.tsx` (props interface ~line 11-44; `handlePartDoubleClick`; part render block ~line 619-633)
- Modify: `packages/playground/src/App.tsx` (pass `onUpdateAttrs` to `<EditorCanvas>` ~line 2423)
- Test: `packages/ui/src/editor/EditorCanvas.note.test.tsx`

**Interfaces:**
- Consumes: existing `handlePartAttrChange` in App.tsx (`(partId, attrs) => editor.updateAttrs(...)`).
- Produces: new optional prop `onUpdateAttrs?: (id: string, attrs: Record<string, string>) => void` on `EditorCanvas`; local edit state `editingNoteId: string | null`.

- [ ] **Step 1: Write the failing test**

```tsx
// packages/ui/src/editor/EditorCanvas.note.test.tsx
import { describe, it, expect, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import { EditorCanvas } from './EditorCanvas';
import { createEmptyDiagram, type Diagram } from './types';

function stateWithNote(): { diagram: Diagram } & Record<string, unknown> {
  return {
    diagram: {
      ...createEmptyDiagram('stm32f103'),
      parts: [{ id: 'note', type: 'note', x: 50, y: 50, rotate: 0, attrs: { text: 'hello' } }],
    },
    selectedIds: new Set<string>(),
    wireInProgress: null,
    undoStack: [],
    redoStack: [],
  };
}

const noop = () => {};
const handlers = {
  onMovePart: noop, onSelect: noop, onStartWire: noop, onCompleteWire: noop,
  onCancelWire: noop, onDeleteWire: noop,
};

describe('EditorCanvas note inline edit', () => {
  it('double-clicking a note enters edit mode and commits on blur', () => {
    const onUpdateAttrs = vi.fn();
    const { container } = render(
      // @ts-expect-error partial state shape is sufficient for this render
      <EditorCanvas state={stateWithNote()} onUpdateAttrs={onUpdateAttrs} {...handlers} />,
    );
    const noteGroup = container.querySelector('[data-part-id="note"]')!;
    fireEvent.doubleClick(noteGroup);
    const editable = container.querySelector('[data-note-editor="note"]') as HTMLElement;
    expect(editable).toBeTruthy();
    editable.textContent = 'edited';
    fireEvent.blur(editable);
    expect(onUpdateAttrs).toHaveBeenCalledWith('note', { text: 'edited' });
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/ui && npx vitest run src/editor/EditorCanvas.note.test.tsx`
Expected: FAIL — no `data-note-editor` element; `onUpdateAttrs` prop unused.

- [ ] **Step 3: Add the prop and edit state**

In `EditorCanvas.tsx` props interface (near the other `on*` callbacks):

```ts
  onUpdateAttrs?: (id: string, attrs: Record<string, string>) => void;
```

Add it to the destructured props, and add local state near the other `useState` hooks:

```ts
  const [editingNoteId, setEditingNoteId] = useState<string | null>(null);
```

- [ ] **Step 4: Branch the double-click handler for notes**

In `handlePartDoubleClick`, after `if (!def) return;`, add:

```ts
      if (part.type === 'note') {
        setEditingNoteId(part.id);
        return;
      }
```

- [ ] **Step 5: Add a stable hook + editor overlay in the part render block**

In the part `<g>` (the one with `onDoubleClick={...}` ~line 620), add `data-part-id={part.id}` to the `<g>`. Then, inside that group, when `editingNoteId === part.id`, render a `<foreignObject>` with a `contentEditable` div instead of (over) the static note. Replace the `{def.render(...)}` usage for notes with an edit overlay:

```tsx
            <g transform={`scale(${sc}) rotate(${part.rotate}, ${def.width / 2}, ${def.height / 2})`}>
              {part.type === 'note' && editingNoteId === part.id ? (
                <foreignObject x={0} y={0} width={def.width} height={1} overflow="visible">
                  <div
                    xmlns="http://www.w3.org/1999/xhtml"
                    data-note-editor={part.id}
                    contentEditable
                    suppressContentEditableWarning
                    ref={(el) => { if (el && el.textContent !== (part.attrs.text ?? '')) el.textContent = part.attrs.text ?? ''; el?.focus(); }}
                    onBlur={(e) => {
                      onUpdateAttrs?.(part.id, { text: e.currentTarget.textContent ?? '' });
                      setEditingNoteId(null);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === 'Escape') { e.preventDefault(); (e.currentTarget as HTMLElement).blur(); }
                    }}
                    style={{
                      width: `${def.width}px`, boxSizing: 'border-box', padding: '12px',
                      background: '#fffdf2', border: '1.5px solid #F5B642', borderRadius: '8px',
                      font: "12px/1.45 -apple-system, 'Segoe UI', sans-serif", color: '#4a3f1e',
                      whiteSpace: 'pre-wrap', wordBreak: 'break-word', outline: 'none',
                    }}
                  />
                </foreignObject>
              ) : (
                def.render(part.attrs, compState)
              )}
              {def.pins.map((pin: PinDef) => {
```

(The `def.pins.map(...)` block and everything after it stays unchanged.)

- [ ] **Step 6: Wire the prop in App.tsx**

In `packages/playground/src/App.tsx`, on the `<EditorCanvas>` element (~line 2423), add:

```tsx
                onUpdateAttrs={handlePartAttrChange}
```

- [ ] **Step 7: Run test to verify it passes**

Run: `cd packages/ui && npx vitest run src/editor/EditorCanvas.note.test.tsx`
Expected: PASS.

- [ ] **Step 8: Run the existing EditorCanvas tests for regressions**

Run: `cd packages/playground && npx vitest run src/mobile/editor-canvas-run-mode.test.tsx`
Expected: PASS (no regressions from the `data-part-id` / render change).

- [ ] **Step 9: Commit**

```bash
git add packages/ui/src/editor/EditorCanvas.tsx \
        packages/ui/src/editor/EditorCanvas.note.test.tsx \
        packages/playground/src/App.tsx
git commit -m "feat(editor): double-click inline editing for notes"
```

---

### Task 4: Seed description notes on every lab + guard inertness

**Files:**
- Modify: `packages/playground/src/App.tsx` (`makeStarterDiagram` ~line 140; add `LAB_NOTES` map + note injection)
- Test: `packages/playground/src/lab-notes.test.ts`

**Interfaces:**
- Consumes: `BOARD_CONFIGS` from `./bundled-configs`, `makeStarterDiagram` (exported), `diagramToConfig` from `@labwired/board-config`.
- Produces: `LAB_NOTES: Record<string, string>`; every non-hidden `kind: 'lab'` config's starter diagram contains exactly one `type: 'note'` part.

- [ ] **Step 1: REVIEW GATE — present the blurb draft to the user**

Show the user the full `LAB_NOTES` draft below (all 20 entries) as a list, apply their edits, and only then proceed. Draft:

```ts
const LAB_NOTES: Record<string, string> = {
  'ntc-thermistor-lab':
    'NTC 3950 thermistor on the STM32F103 ADC. The Steinhart–Hart equation turns raw ADC counts into °C.\nTry: drag the temperature slider and watch the ADC count and computed temperature track it.',
  'neo6m-gps-lab':
    'NEO-6M GPS over UART. Real NMEA sentences stream in and are parsed live.\nTry: Run and watch live position and satellite data decode.',
  'quectel-bg770a-lab':
    'Quectel BG770A LTE-M / NB-IoT modem over UART, with a byte-exact AT-command surface (MQTT/HTTP/GPS state machines).\nTry: Run and watch the firmware drive the modem through its AT sequence.',
  'ssd1306-hello-lab':
    'SSD1306 128×64 OLED over I²C. The firmware draws into a framebuffer the panel renders pixel-for-pixel.\nTry: Run and watch the display paint.',
  'nokia5110-invaders-lab':
    'Nokia 5110 (PCD8544) LCD + ultrasonic sensor on the STM32L476 — a tiny Space-Invaders-style demo.\nTry: Run, then drag the distance sensor to steer.',
  'al2205-iolink-dido':
    'An IO-Link digital-input device (AL2205 profile). Speaks the IO-Link wake-up and process-data cycle.\nTry: open the IO-Link analyzer and Run to watch the master/device exchange.',
  'stm32h5-uds-ecu':
    'A minimal automotive diagnostic ECU on the STM32H5, answering UDS (ISO-14229) requests over FDCAN.\nTry: open the UDS analyzer and Run to send services and read responses.',
  'f103-uds-ecu':
    'UDS diagnostic ECU on the STM32F103 over bxCAN — the “fixed” reference build where every service answers correctly.\nTry: open the UDS analyzer and Run.',
  'f103-uds-ecu-broken':
    'The same F103 UDS ECU with a deliberate negative-response-code bug (-DBROKEN_NCR). A debugging exercise.\nTry: open the UDS analyzer, Run, and spot where it answers wrong vs the fixed build.',
  'bme280-weather-lab':
    'BME280 temperature / humidity / pressure sensor over I²C.\nTry: Run and watch the three environmental readings update.',
  'ili9341-tft-lab':
    'ILI9341 240×320 RGB565 TFT over SPI. The firmware pushes a live color framebuffer.\nTry: Run and watch the panel render in color.',
  'epaper-tricolor-lab':
    'Waveshare 2.9" tri-color (SSD1680) e-paper over SPI. The same firmware ELF flashes to a real NUCLEO-F103 + panel for digital-twin comparison.\nTry: Run and watch the e-paper refresh.',
  'esp32-epaper-lab':
    'An ESP32 driving a tri-color e-paper panel over SPI.\nTry: Run and watch the ESP32 boot and paint the display.',
  'labwired-ereader':
    'An ESP32 e-reader sketch (Arduino .ino, unmodified) driving an e-paper page, ROM-booted.\nTry: Run and page through the reader.',
  'max31855-thermocouple-lab':
    'MAX31855 K-type thermocouple amplifier over SPI, with cold-junction compensation.\nTry: drag the temperature input and watch the converted reading.',
  'mpu6050-sensor-lab':
    'MPU6050 6-axis IMU (accelerometer + gyro) over I²C.\nTry: Run and watch the motion axes update.',
  'vl53l1x-tof-lab':
    'VL53L1X time-of-flight distance sensor over I²C.\nTry: drag the distance and watch the ranging value follow.',
  'adxl345-sensor-lab':
    'ADXL345 3-axis accelerometer over I²C.\nTry: Run and watch the acceleration axes update.',
  'nrf52840-ble-lab':
    'Two nRF52840s on one canvas — a sensor node and a collector — talking over simulated BLE (no wires; they meet on the air).\nTry: Run and watch the sensor advertise and the collector receive.',
  'nrf52840-proximity-lab':
    'An nRF52840 reading a proximity sensor and reporting over BLE.\nTry: Run and watch proximity events broadcast.',
};
```

- [ ] **Step 2: Write the failing test**

```ts
// packages/playground/src/lab-notes.test.ts
import { describe, it, expect } from 'vitest';
import { BOARD_CONFIGS } from './bundled-configs';
import { makeStarterDiagram, LAB_NOTES } from './App';

const visibleLabs = BOARD_CONFIGS.filter((c) => c.kind === 'lab' && !c.hidden);

describe('lab description notes', () => {
  it('every visible lab seeds exactly one note part with non-empty text', () => {
    for (const cfg of visibleLabs) {
      const diagram = makeStarterDiagram(cfg);
      const notes = diagram.parts.filter((p) => p.type === 'note');
      expect(notes, `${cfg.boardId} note count`).toHaveLength(1);
      expect(notes[0].attrs.text?.trim().length, `${cfg.boardId} note text`).toBeGreaterThan(0);
    }
  });

  it('LAB_NOTES has an entry for every visible lab and no orphan keys', () => {
    const labIds = new Set(visibleLabs.map((c) => c.boardId));
    for (const id of labIds) expect(LAB_NOTES[id], `missing note for ${id}`).toBeDefined();
    for (const key of Object.keys(LAB_NOTES)) expect(labIds.has(key), `orphan note key ${key}`).toBe(true);
  });

  it('bare (non-lab) boards seed no note', () => {
    const bare = BOARD_CONFIGS.find((c) => c.kind !== 'lab');
    if (bare) {
      const diagram = makeStarterDiagram(bare);
      expect(diagram.parts.some((p) => p.type === 'note')).toBe(false);
    }
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd packages/playground && npx vitest run src/lab-notes.test.ts`
Expected: FAIL — `LAB_NOTES` is not exported; no note parts seeded.

- [ ] **Step 4: Add `LAB_NOTES` and inject the note**

In `App.tsx`, add the `LAB_NOTES` map (approved text from Step 1) above `makeStarterDiagram`, exported:

```ts
export const LAB_NOTES: Record<string, string> = { /* approved entries */ };
```

Add a helper that appends the note to any diagram, then call it at every `return` in `makeStarterDiagram` — simplest: wrap the function so the single final `return` path adds it. Since `makeStarterDiagram` has many early returns, instead refactor the body to compute `diagram` then return `withLabNote(config, diagram)`:

```ts
function withLabNote(config: BoardConfig, diagram: Diagram): Diagram {
  if (config.kind !== 'lab' || config.hidden) return diagram;
  const text = LAB_NOTES[config.boardId];
  if (!text) return diagram;
  // Banner above the circuit. diagramBounds fits all parts, so negative y is safe.
  const note: Part = { id: 'note', type: 'note', x: 100, y: -150, rotate: 0, attrs: { text } };
  return { ...diagram, parts: [note, ...diagram.parts] };
}
```

Apply it at the function's return points. To avoid touching every early `return`, change each `return { ... }` to `return withLabNote(config, { ... })`. (There are ~18 return sites; update each. The MCU-only fallback `return` at the end also wraps.)

- [ ] **Step 5: Run test to verify it passes**

Run: `cd packages/playground && npx vitest run src/lab-notes.test.ts`
Expected: PASS (3 tests).

- [ ] **Step 6: Guard test — note is inert to config + validation**

Add to the same file:

First find how `diagramToConfig` is actually called in this codebase:

Run: `grep -rn "diagramToConfig(" packages/playground/src packages/ui/src | grep -v test`

Then mirror that exact call signature. The assertion that matters is that the note part id never appears in the generated config:

```ts
import { diagramToConfig } from '@labwired/board-config';

it('a note never contributes a board_io binding', () => {
  const cfg = visibleLabs[0];
  const diagram = makeStarterDiagram(cfg);
  // Call diagramToConfig with the same arguments used in App.tsx (see grep above).
  const generated = diagramToConfig(diagram);
  const json = JSON.stringify(generated ?? {});
  expect(json.includes('"note"')).toBe(false);
});
```

If `diagramToConfig` requires more arguments, supply them as App.tsx does; do not invent a signature.

- [ ] **Step 7: Run the full playground suite for regressions**

Run: `cd packages/playground && npx vitest run src/starterDiagram.test.ts src/lab-notes.test.ts`
Expected: PASS. If `starterDiagram.test.ts` snapshots diagrams, update snapshots intentionally (`-u`) only after confirming the only delta is the added note part.

- [ ] **Step 8: Commit**

```bash
git add packages/playground/src/App.tsx packages/playground/src/lab-notes.test.ts
git commit -m "feat(playground): seed description note on every example lab"
```

---

### Task 5: Visual verification in the running app

**Files:** none (manual verification per the "Actually use what you ship" rule).

- [ ] **Step 1: Build the UI package the playground consumes**

Run: `cd packages/ui && npm run build` (or the workspace's UI build) so the playground picks up the new component. If the dev server aliases `@labwired/ui` to source (see dev-preview setup), skip this.

- [ ] **Step 2: Start the dev preview**

Run (background): `cd packages/playground && VITE_DISABLE_AUTH=1 npm run dev`
Then drive it with chrome-devtools MCP (navigate to the printed localhost URL).

- [ ] **Step 3: Verify the note feature end to end**

- Open 3-4 example labs (e.g. `?board=ntc-thermistor-lab`, `ssd1306-hello-lab`, `stm32h5-uds-ecu`, `nrf52840-ble-lab`). Confirm the description note shows, reads correctly, and does NOT overlap the MCU/peripherals/wires. Take a screenshot of each.
- From the Tools palette, drag a new Note onto the canvas. Double-click it, type, click away — confirm the text commits and persists. Confirm the PropertyPanel textarea edits the same note.
- Press Run on one lab — confirm the sim still boots (note is inert).

- [ ] **Step 4: Report results with screenshots; tune note coordinates if any overlap**

If a note overlaps parts on any lab, adjust that lab's note `x`/`y` (or the default in `withLabNote`) and re-screenshot. Commit any coordinate tweaks:

```bash
git add packages/playground/src/App.tsx
git commit -m "fix(playground): position lab notes clear of circuit"
```

---

### Task 6: Finish the branch

- [ ] **Step 1: Run the relevant test suites green**

Run: `cd packages/ui && npx vitest run src/editor` and `cd packages/playground && npx vitest run src/lab-notes.test.ts src/starterDiagram.test.ts`
Expected: all PASS.

- [ ] **Step 2: Open a PR to `main`**

Use the `superpowers:finishing-a-development-branch` skill. PR targets `main` (no `develop` branch in this repo). Title: "Canvas notes + lab descriptions". Body summarizes the feature and links any tracking issue if one exists.
