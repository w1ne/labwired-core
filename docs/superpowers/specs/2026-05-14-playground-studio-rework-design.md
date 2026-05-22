# Playground "Studio" Rework — Design Spec

**Date:** 2026-05-14
**Status:** Approved (design); spec under user review before plan
**Anchor reference:** Tinkered.ai
**Approach:** Studio (empty dark canvas + hero prompt + chip-row of starter labs)
**Companion plans:** [Device Library Phase 1](../plans/2026-05-14-playground-device-library-phase1.md), [Wokwi Import Compat](../../strategy/wokwi-import-compat.md)

---

## 1. Problem

Today's playground at `app.labwired.com/playground/` is functionally rich (drag-drop canvas, 24 components, wire routing, code editor, multi-tab debug surfaces, sharing, embedding) but visually feels prototype-grade and is not differentiated against Wokwi or Tinkered.ai. The user verdict: *"UI needs rework — first class, fast, pretty, usable, copy from the best."*

The implementation cost of the Device Library Phase 1 work landing on the current shell would mean shipping new devices into a UI that already feels off. We rebuild the shell now so every new device thereafter inherits the production-grade chrome.

## 2. Non-goals (v1)

- **Light theme.** Setting placeholder ships; no light tokens.
- **Mobile editing.** Mobile view is read-only — can run demos, cannot drop parts or draw wires.
- **AI assist behind the prompt bar.** UI affordances ship from day one (slash command, Tab toggle); functional codegen via Claude is a follow-up plan.
- **3D / isometric canvas.** Stays 2D SVG.
- **Custom user-uploaded SVG components.** Phase 2.
- **Realtime collaboration.** Out of scope indefinitely.
- **Light/dark sync with OS.** Out of scope.

## 3. Visual system

### 3.1 Color tokens

```
--lw-bg-base       #0A0B0F   /* page background */
--lw-bg-surface    #13151B   /* default panel surface */
--lw-bg-elevated   #1A1D26   /* modal, dropdown, tooltip */
--lw-bg-canvas     #0E1015   /* canvas backdrop */

--lw-fg-primary    #F2F4F9   /* default text */
--lw-fg-secondary  #9098A8   /* secondary text, labels */
--lw-fg-tertiary   #5A6178   /* muted text, placeholders */

--lw-border        #262A33   /* default border 1px */
--lw-border-strong #363B46   /* emphasized border */
--lw-highlight     rgba(255,255,255,0.04)  /* inset top highlight on glass */

--lw-accent        #5B9DFF   /* primary action, focus, runs */
--lw-accent-hover  #7DB1FF
--lw-accent-soft   rgba(91,157,255,0.12)

--lw-magenta       #F062B8   /* selection, live indicators */
--lw-magenta-soft  rgba(240,98,184,0.14)

--lw-success       #3DD68C
--lw-warning       #F5B642
--lw-danger        #F2545B

--lw-pin-power     #F5B642   /* 3v3/5v wire color */
--lw-pin-gnd       #5A6178   /* gnd wire color */
--lw-pin-i2c-sda   #3DD68C   /* SDA wire */
--lw-pin-i2c-scl   #5B9DFF   /* SCL wire */
--lw-pin-spi-mosi  #F062B8
--lw-pin-spi-miso  #B07BFF
--lw-pin-spi-sck   #5B9DFF
--lw-pin-data      #F2F4F9   /* fallback */
```

### 3.2 Glass card recipe

```css
.lw-glass {
  background: rgba(19, 21, 27, 0.72);
  backdrop-filter: blur(20px) saturate(160%);
  border: 1px solid var(--lw-border);
  box-shadow:
    inset 0 1px 0 var(--lw-highlight),
    0 24px 48px -16px rgba(0, 0, 0, 0.48);
  border-radius: 12px;
}
```

**Reserved for:** hero prompt bar, inspector card, floating sim dock, ⌘K palette, top-chrome dropdowns. Flat surfaces everywhere else (no overuse).

### 3.3 Typography

- **UI:** Inter, 13px / 1.4. Headings use weight 600, not larger size. One exception: hero prompt placeholder is 16px.
- **Mono:** JetBrains Mono, 13px / 1.5. For code, register values, serial output, pin labels on canvas.
- **Two type sizes total in the UI.** Hierarchy via weight + color, not size.

### 3.4 Motion

- Library: `framer-motion`
- Tokens: `easeOut` cubic-bezier(0.16, 1, 0.3, 1), durations 120ms (micro) / 220ms (panel) / 320ms (modal)
- Drag spring: `{ stiffness: 400, damping: 30 }`
- **Banned:** bounces, parallax, scroll-jacking, long staggered entrances. Snappy, not flashy.
- Reduced-motion: respect `prefers-reduced-motion`. All transitions degrade to opacity-only.

### 3.5 Iconography

- Font Awesome 6 (already in landing page) for the top chrome. **Inline SVG** for everything else — no icon font for canvas/inspector. Stroke width 1.5px, 16px / 20px / 24px sizes only.

## 4. Layout regions

```
┌────────────────────────────────────────────────────────────────────┐
│ ⚡ LabWired  ›  STM32F103 Blinky        ⌘K [── search ──]      Dev │  44px top chrome (translucent)
│                                                          👤  Share │
├────────────────────────────────────────────────────────────────────┤
│ ◂                                                                  │
│ ▎palette                  [ canvas / dark grid / full bleed ]      │
│ ▎(slide-out                                                        │
│ ▎  280px)                                          ┌────────────┐  │
│ ▎                                                  │ Inspector  │  │ 320px glass
│ ▎                                                  │ (selection)│  │
│ ▎                                                  └────────────┘  │
│                                                                    │
│                    ┌────────────────────────────────────────┐      │
│                    │ ▶ Run   ⏸   ⏵   ↻      02:14   ● Live  │      │ floating glass dock
│                    └────────────────────────────────────────┘      │
│                                                                    │
│  ┌───────────────────────────────────────────────────────────┐    │
│  │  Dev drawer (off by default)                              │    │
│  │  Serial · Registers · Trace · Memory · YAML               │    │
│  └───────────────────────────────────────────────────────────┘    │
└────────────────────────────────────────────────────────────────────┘
```

### 4.1 Top chrome (44px)

- Left: 18px lightning logo + word "LabWired" → links home, then a `›` breadcrumb showing current project/board name (editable inline).
- Center: ⌘K input. Always visible but inactive-looking — shows placeholder *"Search components, boards, examples…"*. Click or ⌘K opens the full hero prompt (Section 5).
- Right: Avatar (initials placeholder for now, hooks for auth later), Dev toggle pill, Share button (primary accent).
- Background: `rgba(13, 14, 18, 0.6)` + `backdrop-filter: blur(12px)`. Sits over the canvas, no border-bottom.

### 4.2 Palette drawer (slide-out, left edge)

- A 4px-wide always-visible "tab" at `x=0` with a vertical handle pattern. Hover for 300ms or click → drawer slides to 280px wide.
- Tabs at top: **I²C · SPI · UART · Analog · GPIO · Misc**. Active tab has 2px bottom accent underline.
- Search box at top of drawer (`⌘P` focus shortcut). Filters within current tab; empty query shows category.
- Each entry: 32×32 component thumbnail SVG + name + tiny bus-pill (e.g., "I²C 0x53"). Drag to canvas to drop.
- Drawer closes on Esc, click outside, or when a part is dropped.

### 4.3 Canvas

- Full-bleed, behind everything. Background: `--lw-bg-canvas` with a 24px-grid overlay of `rgba(255,255,255,0.025)` 1px lines.
- Infinite pan/zoom. Range: 25% to 400%.
- Components rendered as production SVGs (Section 9). Wires routed with the existing `wire-router.ts`.
- Drop indicator: when dragging from palette, a 8px snap grid lights up faintly and the nearest valid drop region pulses primary accent.
- Empty state: see Section 5.

### 4.4 Inspector glass card

- Appears in `right: 16px; top: 60px` only when a part or wire is selected.
- 320px wide; auto height (max 70vh); scrolls internally.
- Sections (in order): header (icon + name + id + close X), pin map (clickable to highlight on canvas), attributes, **lab widget** (only present for parts that have one — e.g., ADXL345 axis sliders), advanced toggle (reveals register table when on, only inside Dev mode), footer (Duplicate, Delete).
- Multi-select shows a compact group inspector — count + bulk actions only.
- Wire selected: shows endpoints, color, length, "Reroute" / "Delete".

### 4.5 Floating sim dock

- Glass card, `bottom: 16px`, horizontally centered.
- Width: 480px (auto-resizes for longer status text).
- Buttons (left to right): **Run** (primary accent, magenta when live), **Pause**, **Step** (only enabled when paused), **Reset**.
- Right side of dock: run-time `MM:SS`, status pill (Idle / Building / Running / Paused / Halted), separator, "Live" dot pulsing magenta when sim is running.
- Keyboard: Space = Run/Pause, S = Step, R = Reset.

### 4.6 Dev drawer (off by default)

- Toggle in top chrome flips a localStorage flag.
- When on: a 240px drawer slides up from the bottom (above the sim dock). User can resize 160px–60vh by dragging the top handle.
- Tabs: **Serial · Registers · Trace · Memory · YAML**.
- Each tab is implemented today in `packages/ui/src/index.ts` exports; we wrap them in the new dark surface tokens but the components themselves survive.
- Dev mode also adds: Step Cycle button in dock, Set Breakpoint right-click on canvas pins, Snapshot button.

## 5. Hero prompt bar

### 5.1 When it appears

- **On empty canvas:** the prompt bar is the centered hero, vertically positioned at `top: 32vh`. Below it, the chip-row (Section 6).
- **On ⌘K from anywhere:** opens as a modal centered at `top: 18vh` over the canvas; backdrop is `rgba(10, 11, 15, 0.6)` + blur.
- **As input only:** the inactive-looking input in the top chrome reflects the same state. Clicking it opens the modal form.

### 5.2 Visual

```
┌────────────────────────────────────────────────────────────┐
│  ✨   Describe what to build, or pick a starter…           │  16px placeholder
│                                                            │  56px height
└────────────────────────────────────────────────────────────┘
   ↓ on focus, results drawer expands below
┌────────────────────────────────────────────────────────────┐
│  Components                                                │
│   🟢  LED                                          drag    │
│   🔘  Pushbutton                                   drag    │
│  Boards                                                    │
│   🟪  Black Pill (STM32F401CDU6)                  switch   │
│  Examples                                                  │
│   📊  ADXL345 Tilt Lab                            open     │
│  Actions                                                   │
│   ⤴   Share project                                   ⏎   │
└────────────────────────────────────────────────────────────┘
```

### 5.3 Two modes

**Mode 1: Search/command (default)** — typed text runs fuzzy match across four buckets:
- **Components** (drop on canvas) — picking a result drags it from the cursor as if you grabbed it from the palette.
- **Boards** (switch board) — picking loads a new chip + system pair. If the user has a non-empty canvas, a confirmation modal appears: "Switch board? Your current circuit will be saved as a draft."
- **Examples** (labs) — opens the lab as if clicked from the chip-row.
- **Actions** — `Run`, `Share`, `Reset`, `Export YAML`, `Toggle Dev`, `New Project`, etc.

Library: `cmdk`. Up/Down navigates, Enter activates, Esc closes.

**Mode 2: Assist (slash or Tab)** — typing `/` or pressing Tab on an empty input switches to assist mode (placeholder changes to *"Describe a change to your circuit, e.g. 'add an LED on PA5'"*). Functional codegen ships as a follow-up plan; v1 ships:
- The UI affordance (slash and Tab work, mode visibly switches).
- A stub backend route that returns a hardcoded "Sorry, AI assist is coming soon" with a waitlist signup CTA — **so users can discover and self-qualify the feature**.

The full assist mode design + Claude API integration is its own spec, deliberately scoped out here.

### 5.4 Keyboard

- `⌘K` / `Ctrl+K` opens
- `Esc` closes
- `↑` / `↓` navigates results
- `Enter` activates
- `Tab` (on empty input) toggles search ↔ assist mode
- `/` (on empty input) jumps to assist mode

## 6. Starter labs (the chip-row)

Sits under the hero prompt when canvas is empty.

```
[⚡ Blinky]  [📊 ADXL345 Tilt]  [🌡 BME280 Weather]  [📺 OLED Hello]
[📡 GPS Trail]  [🎨 TFT Demo]                 [⌘K all examples ›]
```

| Lab | Status v1 | Hardware | Inspector widget |
|---|---|---|---|
| Blinky | ✅ Ready | STM32F103 + LED on PA5 | LED glow + period slider |
| ADXL345 Tilt | ✅ Ready (uses worktree work) | STM32F103 + ADXL345 over I²C1 | X/Y/Z sliders + line chart |
| BME280 Weather | 🔒 Locked — "Wave 2" | (Device Library Phase 1, Wave 1.3) | Temp/humidity/pressure sliders |
| OLED Hello | 🔒 Locked — "Wave 2" | (Wave 1.4) | Live 128×64 pixel grid |
| GPS Trail | 🔒 Locked — "Wave 3" | (Wave 3) | Lat/lon pad + NMEA console |
| TFT Demo | 🔒 Locked — "Wave 2" | (Wave 2) | 240×320 framebuffer canvas |

Locked tiles render at 60% opacity with a small lock icon. Click → opens a waitlist modal: *"Coming with our [device library Phase 1](https://labwired.com/roadmap). Get notified."* + email input (uses the existing landing page form action if available; otherwise a `mailto:` fallback for v1).

A 7th tile labeled **"⌘K all examples ›"** opens the ⌘K modal scoped to the Examples bucket.

## 7. Interaction model

### 7.1 Drag & drop

- Palette → canvas: drag preview is the component's SVG at 90% opacity; on canvas hover, the drop position snaps to the 8px grid.
- Smart pin-affinity: while dragging, all pins on the MCU compatible with the dragged component's primary bus glow faintly (e.g., dragging an I²C device highlights `PB6`/`PB7` on STM32F103). If the drop position is within 40px of a highlighted pin pair, auto-wire SDA/SCL on drop.
- Drag inside canvas: parts move with cursor; wires reroute live (debounced 16ms).

### 7.2 Wires

- Click on a pin → cursor enters wire-draw mode (cursor changes to crosshair, source pin pulses).
- Click another valid pin → wire is created with color auto-picked from pin role (Section 3.1 `--lw-pin-*` tokens).
- Esc cancels.
- Right-click a wire → context menu: Reroute (re-runs router), Color..., Delete.

### 7.3 Selection

- Single click on part or wire → select + open inspector.
- Shift-click → add to selection.
- Marquee drag on empty canvas → rectangle select.
- Cmd/Ctrl-A → select all.
- Esc → deselect; inspector slides away.

### 7.4 Pan & zoom

- Trackpad: two-finger pan, pinch zoom.
- Mouse: Cmd/Ctrl + scroll = zoom; middle-drag = pan; Space + drag = pan.
- Fit-to-content button (bottom-right of canvas, `bottom: 16px; right: 16px`, 32px square, glass).
- Zoom indicator (small `%` text next to fit button).

### 7.5 Keyboard shortcuts

| Combo | Action |
|---|---|
| ⌘K | Open command palette |
| ⌘S | Save / commit current state |
| ⌘Z / ⌘⇧Z | Undo / redo |
| ⌘D | Duplicate selection |
| ⌘E | Export `system.yaml` |
| Space | Run / Pause sim |
| S | Step (when paused) |
| R | Reset sim *(when sim dock focused)* or Rotate selection 90° *(when canvas focused)* |
| Del / Backspace | Delete selection |
| 0 | Fit to content |
| ⌘0 | Reset zoom to 100% |

## 8. State machine

```
        ┌────────────┐
        │  EMPTY     │  hero prompt + chip-row visible, no parts
        └─────┬──────┘
              │ drop part / pick lab / paste url
              ▼
        ┌────────────┐
        │  AUTHORED  │  parts on canvas, sim idle
        └─────┬──────┘
              │ Run
              ▼
        ┌────────────┐ ◀──────┐
        │  BUILDING  │        │  reset
        └─────┬──────┘        │
              │ ELF ready     │
              ▼               │
        ┌────────────┐        │
        │  RUNNING   │────────┤
        └─────┬──────┘        │
              │ Pause/halt    │
              ▼               │
        ┌────────────┐        │
        │  PAUSED    │────────┘
        └────────────┘
```

`Sim dock status pill` reflects current state. Loading the page in `EMPTY` is the cold-load default unless a `?lab=<id>` URL param or a saved draft exists.

## 9. Component artwork (the "pretty" delivery)

**v1 art pass — top 12 components rebuilt at production quality:**

1. STM32F103 dev board (matched to the Bluepill-ish look)
2. STM32F401CDU6 Black Pill
3. STM32F4 Nucleo board
4. STM32H5 Nucleo-144
5. RP2040 Pico
6. ESP32-S3 Zero
7. LED (multi-color)
8. Pushbutton
9. ADXL345 breakout
10. Potentiometer (panel-mount with knob)
11. SSD1306 OLED 128×64 (will pre-empty until Phase 1 Wave 1.4 ships)
12. Generic resistor / capacitor (banded, recognizable)

Style guide:
- **Recognizable.** Looks like the real part — purple Bluepill PCB, blue Nucleo, white Black Pill silkscreen, etc.
- **Drop shadow.** Cards rest on the canvas grid with a subtle 0/4/12 shadow at 30% opacity.
- **Pin labels.** 9px JetBrains Mono in `--lw-fg-tertiary`, sit just outside the part. On hover, the pin under the cursor + its label brighten.
- **Selected state.** 2px outline in `--lw-magenta` plus a 12px outer glow at 24% opacity.
- **Active state.** Live indicator (pulsing dot, `--lw-success`) on the part when any of its pins are currently transitioning (e.g., LED on).

The remaining 12 existing components (DHT22, LCD1602, keypad, neopixel, etc.) get re-skinned with the new color tokens but keep their current SVG silhouettes; their full art pass is Phase 2.

## 10. Tech stack changes

### 10.1 New dependencies

- `tailwindcss` ^3.4 — replaces the 700-line `playground.css`. Custom theme uses `--lw-*` tokens.
- `framer-motion` ^11 — replaces bespoke CSS transitions, especially for drawers/glass cards.
- `cmdk` ^1 — headless ⌘K palette.
- `clsx` — class merging (tiny).

### 10.2 No additions

- No state management library. Existing `useEditorState` survives.
- No Konva/PixiJS. Canvas remains SVG.
- No new build tooling. Vite + TS unchanged.
- No icon font for the editor. Inline SVG only inside the canvas/inspector. Top chrome keeps Font Awesome.

### 10.3 File structure (new shell)

```
packages/playground/src/
  studio/
    StudioShell.tsx        // top-level region orchestrator (was App.tsx)
    TopChrome.tsx          // 44px header
    HeroPrompt.tsx         // empty-state prompt + chip-row container
    ChipRow.tsx            // starter labs row
    PaletteDrawer.tsx      // slide-out palette
    InspectorCard.tsx      // glass-card right inspector
    SimDock.tsx            // floating sim controls dock
    DevDrawer.tsx          // bottom dev drawer (off by default)
    CommandPalette.tsx     // cmdk wrapper for ⌘K
  hooks/
    useStudioLayout.ts     // shell visibility state, dev mode flag, prompt mode
    useCommandPaletteItems.ts  // composes components/boards/examples/actions
  App.tsx                  // thin: renders <StudioShell /> with providers
```

Outside `studio/`, the existing `BoardPicker.tsx` folds into `TopChrome.tsx`'s breadcrumb. `bundled-configs.ts` survives.

### 10.4 `packages/ui` changes

- New tokens file: `packages/ui/src/styles/tokens.css` defines all `--lw-*` variables.
- `GuidedLab.tsx` and `playground.css` are **deprecated** by this rework. The `Adxl345Visualizer` survives as the lab widget rendered inside the inspector. (No code in `core/` changes.)
- `EditorCanvas` stays — but its bespoke selection ring/hover styles move to the new token system.
- `ComponentPalette.tsx` is replaced by `PaletteDrawer.tsx` (in playground), no longer exported from `@labwired/ui`.

### 10.5 What absolutely does not change

- Anything in `core/` (Rust simulator, peripherals, components, WASM bridge).
- `packages/ui/src/wasm/simulator-bridge.ts`.
- `packages/ui/src/editor/wire-router.ts`, `circuitValidation.ts`, `pin-mapping.ts`, `useEditorState.ts`, `diagramToConfig.ts`.
- Component editor SVG definitions for the 24 existing parts (they get re-skinned with new tokens, not redrawn — except the 12 in Section 9).
- `bundled-configs.ts` board definitions.

## 11. Performance targets

| Metric | Target | Current (est.) |
|---|---|---|
| Cold-load TTI (fast cable) | < 1.2s | ~2.5s |
| Time-to-Blinky (URL → LED visible) | < 2.0s | ~3.5s |
| Canvas pan/zoom (30+ parts) | 60fps | ~50fps |
| Inspector open/close | < 80ms | ~150ms |
| ⌘K open + first results | < 50ms | n/a |
| JS bundle (gzipped, w/o WASM) | < 220kb | ~310kb |

Wins come from: Tailwind purge (vs. 700 lines of hand CSS), code-splitting the WASM bundle behind a dynamic import that fires when Run is pressed (not at load), tree-shaking unused editor components.

## 12. Responsive / mobile

- **Desktop ≥ 1280px:** full Studio as described.
- **Tablet 768–1279px:** palette drawer becomes a bottom sheet (drag-up gesture). Inspector pins to the right at 280px wide. Hero prompt narrows.
- **Mobile ≤ 767px:** **read-only mode.** Canvas renders, parts visible, but drag/wire is disabled. Sim dock at bottom can still Run/Pause/Reset for the loaded lab. Banner at top: *"View only on mobile — open on desktop to edit."*
- The mobile read-only mode reuses existing share-link behavior so demo URLs work everywhere.

## 13. Migration & rollout

- One PR. No feature flag.
- The current playground entry point (`landing_page/playground/index.html` → `app.labwired.com/playground/`) gets swapped in atomically.
- Keep the old shell at `app.labwired.com/playground/legacy/` for two weeks as escape hatch; remove after.
- All saved drafts in `localStorage` keyed by `labwired-{diagram,source}:<boardId>` are forward-compatible (no schema change).
- Embed URLs (`?embed=1&lab=...`) keep working — Studio's empty state shows only the lab, no chrome.

## 14. Accessibility

- All interactive elements reach via Tab. Focus rings use `--lw-accent` 2px offset.
- ⌘K is keyboard-first; drag-drop has a keyboard equivalent (Tab to a palette item → Enter → Tab to a pin on the canvas → Enter to place).
- Contrast: all token combinations pass WCAG AA. Canvas grid is below text contrast threshold (decorative).
- Reduced motion: all `framer-motion` transitions check `prefers-reduced-motion`. Opacity-only fallbacks.
- Screen reader: top chrome uses landmark roles; inspector is an `aside` with `aria-label`; canvas parts have descriptive `aria-label` set by component def.

## 15. Open questions (to resolve before plan)

These are honest unknowns. None block writing the plan, but each is a decision point:

1. **Hero prompt vs. embedded view.** When `?embed=1` is set, should the prompt bar still be visible to embedded users? Current bias: **no, hide hero in embed**, render only the loaded circuit + sim dock.
2. **Chip-row locked-tile waitlist.** Use the existing landing-page form endpoint, or new ConvertKit/Mailerlite hookup? Bias: **reuse existing endpoint if present**, else `mailto:` fallback for v1.
3. **Tailwind vs. CSS variables only.** Tailwind is recommended above, but adds 6KB to the bundle and a config file. Lighter alternative: vanilla CSS with our token file + class utilities written by hand. Bias: **Tailwind**, since utility class names will save the reviewer time during the rework PR.
4. **`cmdk` vs roll our own.** `cmdk` is 8KB gzipped, fully accessible, used by Linear/Vercel/Raycast. Roll-our-own is ~150 LOC. Bias: **`cmdk`** — saves time, well-trodden a11y.
5. **Light theme.** Marked out of scope but Section 4.1 has a no-op toggle slot. Should we omit the toggle entirely until light ships? Bias: **omit until light ships** — no half-features.

## 16. Acceptance criteria

The rework is **shippable** when:

- [ ] First-time visitor lands on `EMPTY` state, sees hero prompt + chip-row, and can click "Blinky" to see the LED running within 2s.
- [ ] ⌘K opens command palette anywhere, fuzzy search returns components / boards / examples / actions, Enter activates.
- [ ] Drag a component from the palette → drop on canvas → wire to MCU → click Run → firmware runs (ADXL345 lab works end-to-end).
- [ ] Inspector glass card shows on selection; ADXL345 widget (X/Y/Z sliders) drives the live simulation.
- [ ] Sim dock floating, Run/Pause/Step/Reset all wire to existing `useSimulationLoop`.
- [ ] Dev mode off by default. Toggle on → drawer slides up with Serial/Registers/Trace/Memory/YAML.
- [ ] Cold load TTI ≤ 1.2s on M2 / fast cable (measured via WebPageTest).
- [ ] All existing labs (Blinky, ADXL345) work without YAML changes.
- [ ] `legacy/` URL serves the old shell for 2 weeks post-deploy.
- [ ] Mobile read-only viewer renders, runs Blinky, refuses to drop parts.

## 17. Plan handoff

After user approval of this spec, the next deliverable is an implementation plan in `docs/superpowers/plans/2026-05-14-playground-studio-rework-plan.md`. The plan will:

- Break the rework into 4–6 implementable tasks (tokens + Tailwind setup, top chrome + palette drawer, hero prompt + chip-row, inspector + sim dock, dev drawer + ⌘K, art pass + perf budget verification).
- Use the same TDD-task-step shape as the ADXL345 plan (`docs/superpowers/plans/2026-05-07-adxl345-sensor-lab-playground.md`).
- Sequence: visual tokens first (so subsequent tasks build against the system), then chrome, then content (palette, hero, inspector), then dev drawer + ⌘K, then art pass.
- Assume the work happens on a new branch `feat/studio-rework` in a fresh worktree under `.worktrees/studio-rework`.

---

## Self-review notes (inline fixes applied)

- **Placeholder scan:** No TBDs. All numeric tokens specified.
- **Internal consistency:** Section 3.1 token list matches all uses elsewhere in the doc.
- **Scope check:** Single plan, single PR. Device Library Phase 1 and AI assist mode are deliberately separate specs.
- **Ambiguity check:** Sections 6 (chip-row), 12 (mobile), 15 (open questions) flagged. None block plan writing — they're bias decisions with explicit recommendations.
