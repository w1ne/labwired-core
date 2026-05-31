# Onboarding a New Chip

A chip is **fully onboarded** only when every item below is complete. The
enforcement tests in `packages/playground/src/bundled-configs.test.ts` (run by
Playground CI) will fail a half-onboarded chip, so nothing merges until all
gates are green.

---

## Touchpoints checklist

Work through these in order. Items marked **(example only)** apply when the
chip ships as a pre-wired demo lab; skip them for bare-board additions.

### 1. Sim model — `core/` repo
- [ ] `core/configs/chips/<chip>.yaml` — full peripheral list (see neighbouring
  YAMLs for the schema; copy the closest chip and adjust addresses / IRQs).
- [ ] Peripheral models — any peripheral type used by the chip must have a
  model in the core engine (`src/peripherals/`). Stub unknown peripherals with
  `type: "stub"` in the YAML until a model lands.
- [ ] E2E test in `core/` — at minimum a `cargo test` that exercises GPIO
  toggle (see `tests/` for examples).

### 2. Demo firmware **(example only)**
- [ ] `core/examples/<lab>/` — firmware source + `system.yaml`.
- [ ] `packages/playground/build-firmware.sh` — add a build entry for the lab.
- [ ] Committed ELF: `packages/playground/public/wasm/demo-<lab>.elf`.

### 3. Board catalog — `packages/playground/src/bundled-configs.ts`
- [ ] Import the chip YAML at the top:
  ```ts
  import chipMyChip from '../../../core/configs/chips/mychip.yaml?raw';
  ```
- [ ] Add a `BOARD_CONFIGS` entry with **all** required fields:
  ```ts
  {
    boardId: 'mychip-devboard',
    chipId: 'mychip',          // <-- must match PIN_MAPS key (see step 4)
    name: 'My Chip DevBoard',
    description: '...',
    arch: 'ARM Cortex-M4',
    chipYaml: chipMyChip,      // <-- must be non-empty
    systemYaml: systemMyChip,
    mcuComponentType: 'my-board-component', // <-- must be in COMPONENT_REGISTRY (see step 6)
  }
  ```
  `chipId` must be the exact key you add to `PIN_MAPS` in the next step.

### 4. Editor pin-mapping — `packages/ui/src/editor/pin-mapping.ts`
- [ ] Add a `<chipId>` key to `PIN_MAPS`.
  - If the GPIO bank is identical or close to an existing chip, alias it with
    an honest comment — this is the established convention:
    ```ts
    mychip: EXISTING_PINS, // <chip family> — same GPIO scheme; TODO: dedicated map
    ```
  - For a genuinely new layout, add a named `const MYCHIP_PINS` table above
    `PIN_MAPS` following the existing `STM32F103_PINS` pattern, then reference it.

  **Enforcement:** the test *"every BOARD_CONFIGS.chipId is a key in PIN_MAPS"*
  will fail CI if this step is skipped.

### 5. Editor chip YAML — `packages/ui/src/editor/diagramToConfig.ts`
- [ ] Add an entry to `CHIP_YAMLS`:
  ```ts
  mychip: `
  name: "..."
  arch: "arm"
  ...
  `,
  ```
  This is used by compile-from-canvas when no `chipYaml` override is supplied
  (e.g. when the user builds their own circuit on the canvas).

### 6. Board component — `packages/ui/src/editor/components/`
- [ ] Create `boards/my-board.tsx` — expose the chip's physical pins as
  `PinDef[]` (follow `boards/stm32-dev.tsx` for a minimal example).
- [ ] Register it in `components/index.ts`:
  ```ts
  import { myBoardComponent } from './boards/my-board';
  // ...
  [myBoardComponent.type, myBoardComponent],
  ```
  The `type` string must match the `mcuComponentType` you used in step 3.

  **Enforcement:** the test *"every BOARD_CONFIGS.mcuComponentType is
  registered in COMPONENT_REGISTRY"* will fail CI if this step is skipped.

### 7. Starter labs **(example only)**
- [ ] Add the new lab to `STARTER_LABS` in
  `packages/playground/src/studio/ChipRow.tsx`:
  ```ts
  { id: 'mychip-mylab', label: 'My Chip Lab', chipLabel: 'My Chip' },
  ```
  Labs left out of `STARTER_LABS` are never surfaced in the Examples picker.

  **Enforcement:** the tests *"every non-hidden kind:\"lab\" board is surfaced
  in STARTER_LABS"* and *"every STARTER_LABS id resolves to a real BOARD_CONFIGS
  entry"* will catch mismatches.

---

## CI enforcement

All invariants above are encoded as blocking tests in:

```
packages/playground/src/bundled-configs.test.ts
```

Run them locally with:

```bash
cd packages/playground
npm ci --ignore-scripts
npm test
```

A PR that adds a chip to `BOARD_CONFIGS` without completing steps 3–6 will
**fail CI** with a descriptive error message listing the offending `boardId` and
`chipId`.

---

## Definition of done

A chip is ready to merge when **all** of the following are true:

- [ ] `cargo test` passes in `core/`
- [ ] `npm test` passes in `packages/playground` (all invariant tests green)
- [ ] `npm run build` succeeds in `packages/ui`
- [ ] The chip appears in the Playground board picker (or Examples, for labs)
  without "Pin X is not available on this board model" errors
- [ ] **(example only)** The demo firmware ELF is committed to
  `packages/playground/public/wasm/` and listed in `demo-assets.json`
