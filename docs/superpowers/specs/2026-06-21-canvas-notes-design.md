# Canvas Notes + Lab Descriptions

**Date:** 2026-06-21
**Branch:** `feat/canvas-notes`

## Problem

Two related gaps in the playground:

1. **No way to annotate a canvas.** Users (and example authors) cannot place free-form
   explanatory text on the playground. The diagram is `parts + wires` only.
2. **Opening an example lab gives no context.** ~20 example labs each have a `description`
   string in `bundled-configs.ts`, but it surfaces only in the project picker — not on the
   canvas. When someone opens a lab they see chips and wires with no idea what it demonstrates
   or what to do.

## Solution overview

Add a generic **note** component — a placeable, editable text card — and use it to ship a
short "what is this / what to do" blurb on every `lab`-kind example.

A note is modeled as a normal `Part` with `type: 'note'`, category `tool` (the same inert
category as the Logic Analyzer and CAN Diagnostic Tool). It has **no pins, no wires, and no
`boardIoKind`**, so it is invisible to `diagramToConfig`, circuit validation, and the
simulator — it cannot affect Run/build. Because it's a `Part`, it serializes into saved
diagrams and shared links for free.

## Components

### 1. Note component — `packages/ui/src/editor/components/note.tsx`

A `ComponentDef`:

- `type: 'note'`, `label: 'Note'`, `category: 'tool'`.
- `pins: []` — no connection points.
- No `boardIoKind`.
- Text stored in `attrs.text`; default a short placeholder ("Double-click to edit").
- Rendered with an SVG `<foreignObject>` wrapping an HTML `<div>` so text wraps naturally
  and the card auto-grows in height. Plain SVG `<text>` cannot wrap.
- Styled as an annotation (soft paper/amber card, subtle rounded border, small label dot) —
  visually distinct from chips so it doesn't read as a circuit component.
- `width` fixed default (≈220 px); height derived from content at render time.

Registered in `packages/ui/src/editor/components/index.ts` under the Tools group.

### 2. Inline editing — `EditorCanvas.tsx` + PropertyPanel

- **Inline:** double-click a note → enters edit mode (a `contentEditable` div inside the
  note's `foreignObject`). Commit on blur or `Esc`; `UPDATE_ATTRS` writes `attrs.text`.
  Editing state is local to `EditorCanvas` (`editingNoteId`), cleared on commit/cancel.
- **Panel fallback:** the PropertyPanel shows a multiline text field for the selected note.
  This requires a new `attrFields` field type `'textarea'` (rendered as a `<textarea>`), so
  editing is also discoverable without knowing the double-click gesture.

### 3. Lab description notes — `makeStarterDiagram(config)` in `App.tsx`

`makeStarterDiagram` already seeds each lab's parts. For every `lab`-kind config it appends a
single note part:

```ts
{ id: 'note', type: 'note', x: <top-left, clear of MCU>, y: <top>, rotate: 0,
  attrs: { text: LAB_NOTES[config.boardId] } }
```

- Note text comes from a `LAB_NOTES: Record<string, string>` map keyed by `boardId`, kept
  next to `makeStarterDiagram`. **The blurbs are the human-reviewed deliverable** (see Review
  Gate). They are short (1–3 lines), distinct from the longer picker `description`: a plain
  "what this demonstrates + what to do" hook.
- Positioned in clear canvas space (default: above/left of the MCU, which seeds at x:100,
  y:100). Exact per-lab offset is tuned during implementation so the note never covers a part
  or wire; the parity test only asserts presence, not coordinates.
- Only `lab`-kind configs get a note. Bare MCU starter boards stay clean.

## Data flow

- Note is a `Part` → flows through the existing reducer (ADD/MOVE/DELETE/UPDATE_ATTRS/
  SELECT/UNDO/REDO) and `LOAD_DIAGRAM` with no special-casing.
- Serialization: already covered — `Part` is JSON. Saved projects and shared URLs round-trip
  notes automatically.
- `diagramToConfig` (in `@labwired/board-config`) derives `board_io` from `boardIoKind`
  components; a note has none, so it produces nothing. No simulator impact.

## Error handling / edge cases

- Empty note text → render the placeholder, still selectable/deletable.
- A note in a shared diagram loaded by an older client without the `note` component: the
  canvas renderer must skip unknown part types gracefully (verify current behavior; if it
  throws, guard it).
- Multiple notes on one canvas allowed (user-added). The seeded lab note uses id `note`;
  user-added notes get unique ids like every other part.

## Testing

- **Unit:** a `note` Part serializes/deserializes; `diagramToConfig` ignores it; circuit
  validation/diagnostics emit nothing for it (no wires, no boardIoKind).
- **Snapshot/parity:** every `lab`-kind config in `BOARD_CONFIGS` produces a starter diagram
  with exactly one `type: 'note'` part, and `LAB_NOTES` has an entry for each (no missing/
  orphan keys).
- **Component render:** note renders without throwing for empty and long text.

## Out of scope (YAGNI)

- Resizable note width via drag handle (fixed default width for v1; height auto).
- Rich text / markdown inside notes (plain text only).
- Per-bare-board hint notes.

## Review Gate

Before any commit of the lab blurbs, the full `LAB_NOTES` list (all ~20 labs) is handed to
the user to read and edit. Nothing commits until the text is approved.
