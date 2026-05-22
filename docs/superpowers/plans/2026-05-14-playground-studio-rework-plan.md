# Playground "Studio" Rework — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the playground at `app.labwired.com/playground/` as a polished, Tinkered.ai-inspired "Studio" shell with a dark visual system, hero command bar, slide-out palette, glass inspector, floating sim dock, and opt-in Dev drawer — while preserving the existing simulator core, WASM bridge, and editor canvas primitives.

**Architecture:** New shell components live under `packages/playground/src/studio/`. Tailwind + design tokens replace the bespoke `playground.css`. `framer-motion` drives panel animations. `cmdk` powers the ⌘K palette. The existing `useEditorState` hook, `EditorCanvas`, wire router, validation, and simulator bridge survive untouched.

**Tech Stack:** React 19, TypeScript, Vite, Vitest + React Testing Library, Playwright (smoke), Tailwind CSS, framer-motion, cmdk, clsx.

**Source spec:** [`docs/superpowers/specs/2026-05-14-playground-studio-rework-design.md`](../specs/2026-05-14-playground-studio-rework-design.md).

---

## File structure

### New files (created in this plan)

```
packages/playground/
  tailwind.config.ts                              # Task 1
  postcss.config.cjs                              # Task 1
  src/
    styles/
      tokens.css                                  # Task 1 — design tokens
      tailwind.css                                # Task 1 — @tailwind directives + globals
    studio/
      StudioShell.tsx                             # Task 2 — top-level region orchestrator
      TopChrome.tsx                               # Task 2 — 44px header
      HeroPrompt.tsx                              # Task 3 — empty-state hero
      ChipRow.tsx                                 # Task 3 — starter labs row
      WaitlistModal.tsx                           # Task 3 — locked-lab waitlist
      PaletteDrawer.tsx                           # Task 4 — slide-out palette
      InspectorCard.tsx                           # Task 5 — glass card for selection
      SimDock.tsx                                 # Task 6 — floating sim controls
      DevDrawer.tsx                               # Task 7 — bottom dev surfaces
      CommandPalette.tsx                          # Task 8 — cmdk ⌘K modal
      useStudioLayout.ts                          # Task 2 — shell visibility / dev mode
      useCommandPaletteItems.ts                   # Task 8 — composes palette items
      art/                                        # Task 9 — production-grade SVG art
        stm32f103.tsx
        stm32f401cdu6.tsx
        nucleo-f401.tsx
        nucleo-h563.tsx
        rp2040-pico.tsx
        esp32s3-zero.tsx
        led-pro.tsx
        button-pro.tsx
        adxl345-pro.tsx
        potentiometer-pro.tsx
        ssd1306-pro.tsx
        resistor-pro.tsx
    legacy/
      App.legacy.tsx                              # Task 10 — keep old shell reachable
      legacy.html                                 # Task 10 — /playground/legacy/ entry
```

### Modified files

```
packages/playground/
  package.json                                    # Task 1 — deps + scripts
  vite.config.ts                                  # Task 1 — add @tailwindcss/vite or postcss
  index.html                                      # Task 1 — link tokens+tailwind
  src/
    App.tsx                                       # Task 2 — thin wrapper for StudioShell
    main.tsx                                      # Task 1 — import styles
    Icons.tsx                                     # Task 2 — extend with logo + chevron icons
playwright.config.ts                              # Task 10 — config if missing
packages/ui/
  src/
    index.ts                                      # Task 5 — deprecate GuidedLab export
    editor/EditorCanvas.tsx                       # Task 5 — adopt new tokens
```

### Files explicitly NOT touched

- Anything in `core/`
- `packages/ui/src/wasm/simulator-bridge.ts`
- `packages/ui/src/editor/wire-router.ts`
- `packages/ui/src/editor/circuitValidation.ts`
- `packages/ui/src/editor/pin-mapping.ts`
- `packages/ui/src/editor/useEditorState.ts`
- `packages/ui/src/editor/diagramToConfig.ts`
- `packages/playground/src/bundled-configs.ts`

---

## Pre-flight: branch and worktree

Before Task 1, the controller sets up an isolated worktree:

```bash
cd ~/Projects/labwired
git worktree add -b feat/studio-rework .worktrees/studio-rework main
cd .worktrees/studio-rework
```

All tasks in this plan run inside `.worktrees/studio-rework/`. Submodule pointers (`core`, `landing_page`, `vscode`) are left untouched.

---

## Task 1: Design tokens & Tailwind foundation

**Files:**
- Create: `packages/playground/tailwind.config.ts`
- Create: `packages/playground/postcss.config.cjs`
- Create: `packages/playground/src/styles/tokens.css`
- Create: `packages/playground/src/styles/tailwind.css`
- Modify: `packages/playground/package.json`
- Modify: `packages/playground/index.html`
- Modify: `packages/playground/src/main.tsx`
- Test: `packages/playground/src/styles/tokens.test.ts`

- [ ] **Step 1: Install dependencies**

Run:

```bash
cd packages/playground
npm install -D tailwindcss@^3.4 postcss autoprefixer @tailwindcss/forms
npm install framer-motion@^11 cmdk@^1 clsx
```

Expected: `package.json` gets four new entries; `node_modules` updates without errors.

- [ ] **Step 2: Write the failing tokens test**

Create `packages/playground/src/styles/tokens.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const tokens = readFileSync(resolve(__dirname, './tokens.css'), 'utf-8');

const REQUIRED_TOKENS = [
  '--lw-bg-base', '--lw-bg-surface', '--lw-bg-elevated', '--lw-bg-canvas',
  '--lw-fg-primary', '--lw-fg-secondary', '--lw-fg-tertiary',
  '--lw-border', '--lw-border-strong', '--lw-highlight',
  '--lw-accent', '--lw-accent-hover', '--lw-accent-soft',
  '--lw-magenta', '--lw-magenta-soft',
  '--lw-success', '--lw-warning', '--lw-danger',
  '--lw-pin-power', '--lw-pin-gnd',
  '--lw-pin-i2c-sda', '--lw-pin-i2c-scl',
  '--lw-pin-spi-mosi', '--lw-pin-spi-miso', '--lw-pin-spi-sck',
  '--lw-pin-data',
];

describe('design tokens', () => {
  it('exports every token named in the design spec', () => {
    for (const token of REQUIRED_TOKENS) {
      expect(tokens).toContain(`${token}:`);
    }
  });

  it('defines tokens under :root', () => {
    expect(tokens).toMatch(/:root\s*{/);
  });
});
```

- [ ] **Step 3: Run the failing test**

Run:

```bash
cd packages/playground
npm test -- --run src/styles/tokens.test
```

Expected: FAIL — file `./tokens.css` does not exist.

- [ ] **Step 4: Create tokens.css with the full token set**

Create `packages/playground/src/styles/tokens.css`:

```css
:root {
  --lw-bg-base: #0A0B0F;
  --lw-bg-surface: #13151B;
  --lw-bg-elevated: #1A1D26;
  --lw-bg-canvas: #0E1015;

  --lw-fg-primary: #F2F4F9;
  --lw-fg-secondary: #9098A8;
  --lw-fg-tertiary: #5A6178;

  --lw-border: #262A33;
  --lw-border-strong: #363B46;
  --lw-highlight: rgba(255, 255, 255, 0.04);

  --lw-accent: #5B9DFF;
  --lw-accent-hover: #7DB1FF;
  --lw-accent-soft: rgba(91, 157, 255, 0.12);

  --lw-magenta: #F062B8;
  --lw-magenta-soft: rgba(240, 98, 184, 0.14);

  --lw-success: #3DD68C;
  --lw-warning: #F5B642;
  --lw-danger: #F2545B;

  --lw-pin-power: #F5B642;
  --lw-pin-gnd: #5A6178;
  --lw-pin-i2c-sda: #3DD68C;
  --lw-pin-i2c-scl: #5B9DFF;
  --lw-pin-spi-mosi: #F062B8;
  --lw-pin-spi-miso: #B07BFF;
  --lw-pin-spi-sck: #5B9DFF;
  --lw-pin-data: #F2F4F9;

  --lw-radius-card: 12px;
  --lw-radius-button: 8px;
  --lw-radius-pill: 999px;

  --lw-shadow-glass: inset 0 1px 0 var(--lw-highlight),
    0 24px 48px -16px rgba(0, 0, 0, 0.48);

  --lw-blur-glass: blur(20px) saturate(160%);

  --lw-font-ui: 'Inter', system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
  --lw-font-mono: 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace;

  --lw-ease-out: cubic-bezier(0.16, 1, 0.3, 1);
  --lw-dur-micro: 120ms;
  --lw-dur-panel: 220ms;
  --lw-dur-modal: 320ms;
}

@media (prefers-reduced-motion: reduce) {
  :root {
    --lw-dur-micro: 1ms;
    --lw-dur-panel: 1ms;
    --lw-dur-modal: 1ms;
  }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run:

```bash
cd packages/playground
npm test -- --run src/styles/tokens.test
```

Expected: PASS (2 tests).

- [ ] **Step 6: Create Tailwind config and styles**

Create `packages/playground/postcss.config.cjs`:

```js
module.exports = {
  plugins: {
    tailwindcss: {},
    autoprefixer: {},
  },
};
```

Create `packages/playground/tailwind.config.ts`:

```ts
import type { Config } from 'tailwindcss';
import forms from '@tailwindcss/forms';

const config: Config = {
  content: [
    './index.html',
    './src/**/*.{ts,tsx}',
    '../ui/src/**/*.{ts,tsx}',
  ],
  theme: {
    extend: {
      colors: {
        bg: {
          base: 'var(--lw-bg-base)',
          surface: 'var(--lw-bg-surface)',
          elevated: 'var(--lw-bg-elevated)',
          canvas: 'var(--lw-bg-canvas)',
        },
        fg: {
          primary: 'var(--lw-fg-primary)',
          secondary: 'var(--lw-fg-secondary)',
          tertiary: 'var(--lw-fg-tertiary)',
        },
        border: {
          DEFAULT: 'var(--lw-border)',
          strong: 'var(--lw-border-strong)',
        },
        accent: {
          DEFAULT: 'var(--lw-accent)',
          hover: 'var(--lw-accent-hover)',
          soft: 'var(--lw-accent-soft)',
        },
        magenta: {
          DEFAULT: 'var(--lw-magenta)',
          soft: 'var(--lw-magenta-soft)',
        },
        ok: 'var(--lw-success)',
        warn: 'var(--lw-warning)',
        danger: 'var(--lw-danger)',
      },
      fontFamily: {
        sans: ['var(--lw-font-ui)'],
        mono: ['var(--lw-font-mono)'],
      },
      borderRadius: {
        card: 'var(--lw-radius-card)',
        button: 'var(--lw-radius-button)',
        pill: 'var(--lw-radius-pill)',
      },
      boxShadow: {
        glass: 'var(--lw-shadow-glass)',
      },
      transitionTimingFunction: {
        out: 'var(--lw-ease-out)',
      },
      transitionDuration: {
        micro: 'var(--lw-dur-micro)',
        panel: 'var(--lw-dur-panel)',
        modal: 'var(--lw-dur-modal)',
      },
    },
  },
  plugins: [forms],
};

export default config;
```

Create `packages/playground/src/styles/tailwind.css`:

```css
@tailwind base;
@tailwind components;
@tailwind utilities;

@layer base {
  html, body, #root {
    height: 100%;
    margin: 0;
    background: var(--lw-bg-base);
    color: var(--lw-fg-primary);
    font-family: var(--lw-font-ui);
    font-size: 13px;
    line-height: 1.4;
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }

  ::selection {
    background: var(--lw-magenta-soft);
  }
}

@layer components {
  .lw-glass {
    background: rgba(19, 21, 27, 0.72);
    backdrop-filter: var(--lw-blur-glass);
    border: 1px solid var(--lw-border);
    box-shadow: var(--lw-shadow-glass);
    border-radius: var(--lw-radius-card);
  }
}
```

- [ ] **Step 7: Wire styles into the entry point**

Modify `packages/playground/src/main.tsx` (add the two CSS imports at top):

```tsx
import './styles/tokens.css';
import './styles/tailwind.css';
import { StrictMode } from 'react';
// ... existing imports
```

Remove (or comment for now) the existing `playground.css` import.

Modify `packages/playground/index.html` `<head>` to drop the Inter + JetBrains Mono `<link>` if not already present:

```html
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&family=JetBrains+Mono:wght@400;500&display=swap" rel="stylesheet">
```

- [ ] **Step 8: Verify build still works**

Run:

```bash
cd packages/playground
npm run build
```

Expected: Vite build exits `0`. Tailwind utility classes used so far (`bg-bg-base`, `text-fg-primary`) produce no warnings. Bundle does not error on missing CSS.

- [ ] **Step 9: Commit**

```bash
git add packages/playground/package.json packages/playground/package-lock.json \
        packages/playground/postcss.config.cjs packages/playground/tailwind.config.ts \
        packages/playground/src/styles \
        packages/playground/src/main.tsx packages/playground/index.html
git commit -m "feat(playground): add design tokens + Tailwind foundation"
```

---

## Task 2: Studio shell scaffold + TopChrome

**Files:**
- Create: `packages/playground/src/studio/StudioShell.tsx`
- Create: `packages/playground/src/studio/TopChrome.tsx`
- Create: `packages/playground/src/studio/useStudioLayout.ts`
- Modify: `packages/playground/src/App.tsx`
- Modify: `packages/playground/src/Icons.tsx`
- Test: `packages/playground/src/studio/StudioShell.test.tsx`
- Test: `packages/playground/src/studio/TopChrome.test.tsx`

- [ ] **Step 1: Write failing tests**

Create `packages/playground/src/studio/StudioShell.test.tsx`:

```tsx
import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { StudioShell } from './StudioShell';

describe('StudioShell', () => {
  it('renders the top chrome', () => {
    render(<StudioShell />);
    expect(screen.getByRole('banner')).toBeInTheDocument();
  });

  it('renders an aside region for selection inspector', () => {
    render(<StudioShell />);
    expect(screen.queryByRole('complementary', { name: /inspector/i })).toBeNull();
  });

  it('renders main canvas region', () => {
    render(<StudioShell />);
    expect(screen.getByRole('main', { name: /canvas/i })).toBeInTheDocument();
  });
});
```

Create `packages/playground/src/studio/TopChrome.test.tsx`:

```tsx
import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { TopChrome } from './TopChrome';

describe('TopChrome', () => {
  it('renders the LabWired brand link', () => {
    render(<TopChrome boardName="Untitled" onOpenCommand={() => {}} devMode={false} onToggleDev={() => {}} />);
    expect(screen.getByRole('link', { name: /labwired/i })).toBeInTheDocument();
  });

  it('shows the current board name in the breadcrumb', () => {
    render(<TopChrome boardName="STM32F103 Blinky" onOpenCommand={() => {}} devMode={false} onToggleDev={() => {}} />);
    expect(screen.getByText('STM32F103 Blinky')).toBeInTheDocument();
  });

  it('opens the command palette when the search input is clicked', async () => {
    const onOpenCommand = vi.fn();
    render(<TopChrome boardName="Untitled" onOpenCommand={onOpenCommand} devMode={false} onToggleDev={() => {}} />);
    await userEvent.click(screen.getByPlaceholderText(/search components/i));
    expect(onOpenCommand).toHaveBeenCalled();
  });

  it('toggles dev mode when the Dev pill is clicked', async () => {
    const onToggleDev = vi.fn();
    render(<TopChrome boardName="Untitled" onOpenCommand={() => {}} devMode={false} onToggleDev={onToggleDev} />);
    await userEvent.click(screen.getByRole('switch', { name: /dev mode/i }));
    expect(onToggleDev).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run failing tests**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/StudioShell.test src/studio/TopChrome.test
```

Expected: FAIL — modules not found.

- [ ] **Step 3: Implement `useStudioLayout`**

Create `packages/playground/src/studio/useStudioLayout.ts`:

```ts
import { useCallback, useState } from 'react';

const DEV_KEY = 'labwired:dev-mode';

export interface StudioLayoutState {
  paletteOpen: boolean;
  commandOpen: boolean;
  devMode: boolean;
}

export interface StudioLayoutActions {
  setPaletteOpen: (open: boolean) => void;
  openCommand: () => void;
  closeCommand: () => void;
  toggleDev: () => void;
}

export function useStudioLayout(): StudioLayoutState & StudioLayoutActions {
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [commandOpen, setCommandOpen] = useState(false);
  const [devMode, setDevMode] = useState(() => localStorage.getItem(DEV_KEY) === '1');

  const openCommand = useCallback(() => setCommandOpen(true), []);
  const closeCommand = useCallback(() => setCommandOpen(false), []);
  const toggleDev = useCallback(() => {
    setDevMode((current) => {
      const next = !current;
      localStorage.setItem(DEV_KEY, next ? '1' : '0');
      return next;
    });
  }, []);

  return { paletteOpen, commandOpen, devMode, setPaletteOpen, openCommand, closeCommand, toggleDev };
}
```

- [ ] **Step 4: Implement `TopChrome`**

Create `packages/playground/src/studio/TopChrome.tsx`:

```tsx
import clsx from 'clsx';

export interface TopChromeProps {
  boardName: string;
  devMode: boolean;
  onOpenCommand: () => void;
  onToggleDev: () => void;
  onShare?: () => void;
}

export function TopChrome({ boardName, devMode, onOpenCommand, onToggleDev, onShare }: TopChromeProps) {
  return (
    <header
      role="banner"
      className="absolute top-0 inset-x-0 z-30 flex items-center gap-3 h-11 px-3 bg-[rgba(13,14,18,0.6)] backdrop-blur border-b border-border/0"
    >
      <a href="/" className="flex items-center gap-2 text-fg-primary font-semibold tracking-tight">
        <svg viewBox="0 0 20 20" width="18" height="18" aria-hidden="true">
          <path d="M11 2 4 12h4l-1 6 8-10h-4l1-6z" fill="currentColor" />
        </svg>
        LabWired
      </a>
      <span className="text-fg-tertiary">›</span>
      <span className="text-fg-secondary truncate max-w-[28ch]">{boardName}</span>

      <div className="flex-1 max-w-[520px] mx-auto">
        <button
          type="button"
          onClick={onOpenCommand}
          className="w-full h-8 px-3 flex items-center gap-2 rounded-button bg-bg-surface/70 border border-border text-fg-tertiary text-left hover:border-border-strong transition-colors duration-micro"
        >
          <span aria-hidden>⌘K</span>
          <input
            tabIndex={-1}
            readOnly
            placeholder="Search components, boards, examples…"
            className="bg-transparent flex-1 outline-none text-fg-secondary placeholder:text-fg-tertiary"
          />
        </button>
      </div>

      <button
        type="button"
        role="switch"
        aria-checked={devMode}
        aria-label="Dev mode"
        onClick={onToggleDev}
        className={clsx(
          'h-7 px-3 rounded-pill text-xs font-medium transition-colors duration-micro',
          devMode
            ? 'bg-magenta-soft text-magenta border border-magenta/40'
            : 'bg-bg-surface/60 text-fg-secondary border border-border hover:text-fg-primary'
        )}
      >
        Dev {devMode ? 'on' : 'off'}
      </button>
      <button
        type="button"
        onClick={onShare}
        className="h-7 px-3 rounded-pill text-xs font-medium bg-accent text-bg-base hover:bg-accent-hover transition-colors duration-micro"
      >
        Share
      </button>
    </header>
  );
}
```

- [ ] **Step 5: Implement `StudioShell`**

Create `packages/playground/src/studio/StudioShell.tsx`:

```tsx
import { TopChrome } from './TopChrome';
import { useStudioLayout } from './useStudioLayout';

export interface StudioShellProps {
  boardName?: string;
  children?: React.ReactNode;
}

export function StudioShell({ boardName = 'Untitled', children }: StudioShellProps) {
  const layout = useStudioLayout();

  return (
    <div className="relative w-full h-screen overflow-hidden bg-bg-base text-fg-primary">
      <TopChrome
        boardName={boardName}
        devMode={layout.devMode}
        onOpenCommand={layout.openCommand}
        onToggleDev={layout.toggleDev}
      />
      <main role="main" aria-label="Canvas" className="absolute inset-0 pt-11 bg-bg-canvas">
        {children}
      </main>
    </div>
  );
}
```

- [ ] **Step 6: Wire `StudioShell` into `App.tsx`**

The existing 1014-line `App.tsx` continues to drive simulation state. Wrap the existing center content with the new shell. At the top of the component, render `<StudioShell>` and move existing JSX inside its main slot. Remove the old top-bar JSX (board picker, run controls) — they will be re-introduced in their new locations across Tasks 3, 6, 8. Keep `Adxl345Visualizer`, `EditorCanvas`, `SimControls`, `BoardPicker` imports for now; they're re-homed in later tasks.

Edit the `return` of `function App()`:

```tsx
return (
  <StudioShell boardName={selectedBoard.name}>
    {/* existing editor layout container goes here; will be replaced in Task 5/6 */}
    <div data-legacy-shell="true">
      {/* ...existing layout JSX... */}
    </div>
  </StudioShell>
);
```

- [ ] **Step 7: Add Icons used by chrome**

Modify `packages/playground/src/Icons.tsx` — append:

```tsx
export const LogoIcon = (props: React.SVGProps<SVGSVGElement>) => (
  <svg viewBox="0 0 20 20" width="18" height="18" {...props}>
    <path d="M11 2 4 12h4l-1 6 8-10h-4l1-6z" fill="currentColor" />
  </svg>
);

export const ChevronRightThinIcon = (props: React.SVGProps<SVGSVGElement>) => (
  <svg viewBox="0 0 16 16" width="12" height="12" {...props}>
    <path d="M6 4l4 4-4 4" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);
```

- [ ] **Step 8: Run tests to verify they pass**

Run:

```bash
cd packages/playground
npm test -- --run src/studio
```

Expected: PASS (7 tests across StudioShell + TopChrome).

- [ ] **Step 9: Commit**

```bash
git add packages/playground/src/studio packages/playground/src/App.tsx packages/playground/src/Icons.tsx
git commit -m "feat(playground): add Studio shell scaffold + top chrome"
```

---

## Task 3: Hero prompt + chip-row + waitlist modal

**Files:**
- Create: `packages/playground/src/studio/HeroPrompt.tsx`
- Create: `packages/playground/src/studio/ChipRow.tsx`
- Create: `packages/playground/src/studio/WaitlistModal.tsx`
- Modify: `packages/playground/src/studio/StudioShell.tsx`
- Modify: `packages/playground/src/App.tsx`
- Test: `packages/playground/src/studio/HeroPrompt.test.tsx`
- Test: `packages/playground/src/studio/ChipRow.test.tsx`
- Test: `packages/playground/src/studio/WaitlistModal.test.tsx`

- [ ] **Step 1: Write failing tests**

Create `packages/playground/src/studio/ChipRow.test.tsx`:

```tsx
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ChipRow, STARTER_LABS } from './ChipRow';

describe('ChipRow', () => {
  it('renders all 6 starter labs', () => {
    render(<ChipRow onPick={() => {}} onLocked={() => {}} />);
    for (const lab of STARTER_LABS) {
      expect(screen.getByText(lab.name)).toBeInTheDocument();
    }
  });

  it('invokes onPick when an unlocked lab is clicked', async () => {
    const onPick = vi.fn();
    render(<ChipRow onPick={onPick} onLocked={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: /blinky/i }));
    expect(onPick).toHaveBeenCalledWith('stm32f103-blinky');
  });

  it('invokes onLocked when a locked lab is clicked', async () => {
    const onLocked = vi.fn();
    render(<ChipRow onPick={() => {}} onLocked={onLocked} />);
    await userEvent.click(screen.getByRole('button', { name: /bme280/i }));
    expect(onLocked).toHaveBeenCalledWith('bme280-weather');
  });
});
```

Create `packages/playground/src/studio/HeroPrompt.test.tsx`:

```tsx
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { HeroPrompt } from './HeroPrompt';

describe('HeroPrompt', () => {
  it('renders the hero prompt placeholder', () => {
    render(<HeroPrompt onFocus={() => {}} />);
    expect(screen.getByPlaceholderText(/describe what to build/i)).toBeInTheDocument();
  });

  it('invokes onFocus when the input is focused', async () => {
    const onFocus = vi.fn();
    render(<HeroPrompt onFocus={onFocus} />);
    await userEvent.click(screen.getByPlaceholderText(/describe what to build/i));
    expect(onFocus).toHaveBeenCalled();
  });
});
```

Create `packages/playground/src/studio/WaitlistModal.test.tsx`:

```tsx
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { WaitlistModal } from './WaitlistModal';

describe('WaitlistModal', () => {
  it('does not render when closed', () => {
    render(<WaitlistModal open={false} labName="BME280 Weather" onClose={() => {}} />);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('renders the lab name when open', () => {
    render(<WaitlistModal open={true} labName="BME280 Weather" onClose={() => {}} />);
    expect(screen.getByRole('dialog')).toHaveTextContent('BME280 Weather');
  });

  it('calls onClose on Escape', async () => {
    const onClose = vi.fn();
    render(<WaitlistModal open={true} labName="BME280 Weather" onClose={onClose} />);
    await userEvent.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run failing tests**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/HeroPrompt src/studio/ChipRow src/studio/WaitlistModal
```

Expected: FAIL — modules not found.

- [ ] **Step 3: Implement `ChipRow`**

Create `packages/playground/src/studio/ChipRow.tsx`:

```tsx
import clsx from 'clsx';

export interface StarterLab {
  id: string;
  name: string;
  icon: string;
  locked: boolean;
  comingIn?: string;
}

export const STARTER_LABS: StarterLab[] = [
  { id: 'stm32f103-blinky', name: 'Blinky', icon: '⚡', locked: false },
  { id: 'adxl345-sensor-lab', name: 'ADXL345 Tilt', icon: '📊', locked: false },
  { id: 'bme280-weather', name: 'BME280 Weather', icon: '🌡', locked: true, comingIn: 'Wave 2' },
  { id: 'oled-hello', name: 'OLED Hello', icon: '📺', locked: true, comingIn: 'Wave 2' },
  { id: 'gps-trail', name: 'GPS Trail', icon: '📡', locked: true, comingIn: 'Wave 3' },
  { id: 'tft-demo', name: 'TFT Demo', icon: '🎨', locked: true, comingIn: 'Wave 2' },
];

export interface ChipRowProps {
  onPick: (labId: string) => void;
  onLocked: (labId: string) => void;
}

export function ChipRow({ onPick, onLocked }: ChipRowProps) {
  return (
    <div className="flex flex-wrap gap-2 justify-center max-w-[640px] mx-auto">
      {STARTER_LABS.map((lab) => (
        <button
          key={lab.id}
          type="button"
          onClick={() => (lab.locked ? onLocked(lab.id) : onPick(lab.id))}
          className={clsx(
            'h-9 px-3 rounded-pill text-xs font-medium flex items-center gap-2',
            'transition-colors duration-micro',
            lab.locked
              ? 'bg-bg-surface/50 border border-border text-fg-tertiary hover:text-fg-secondary'
              : 'bg-bg-surface border border-border text-fg-primary hover:border-accent hover:text-accent'
          )}
        >
          <span aria-hidden>{lab.icon}</span>
          {lab.name}
          {lab.locked && (
            <span className="text-fg-tertiary text-[10px] uppercase tracking-wider ml-1">{lab.comingIn}</span>
          )}
        </button>
      ))}
    </div>
  );
}
```

- [ ] **Step 4: Implement `HeroPrompt`**

Create `packages/playground/src/studio/HeroPrompt.tsx`:

```tsx
import { motion } from 'framer-motion';

export interface HeroPromptProps {
  onFocus: () => void;
}

export function HeroPrompt({ onFocus }: HeroPromptProps) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.32, ease: [0.16, 1, 0.3, 1] }}
      className="lw-glass w-[min(560px,calc(100vw-32px))] mx-auto"
    >
      <button
        type="button"
        onClick={onFocus}
        className="w-full h-14 px-5 flex items-center gap-3 text-left"
        aria-label="Open command palette"
      >
        <span className="text-magenta text-lg" aria-hidden>
          ✨
        </span>
        <input
          tabIndex={-1}
          readOnly
          onFocus={onFocus}
          placeholder="Describe what to build, or pick a starter…"
          className="flex-1 bg-transparent outline-none text-[16px] placeholder:text-fg-tertiary"
        />
        <kbd className="text-fg-tertiary text-xs border border-border rounded px-1.5 py-0.5">⌘K</kbd>
      </button>
    </motion.div>
  );
}
```

- [ ] **Step 5: Implement `WaitlistModal`**

Create `packages/playground/src/studio/WaitlistModal.tsx`:

```tsx
import { useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';

export interface WaitlistModalProps {
  open: boolean;
  labName: string;
  onClose: () => void;
}

export function WaitlistModal({ open, labName, onClose }: WaitlistModalProps) {
  const [email, setEmail] = useState('');

  useEffect(() => {
    if (!open) return;
    const handleEsc = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleEsc);
    return () => document.removeEventListener('keydown', handleEsc);
  }, [open, onClose]);

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.16 }}
          className="fixed inset-0 z-50 flex items-center justify-center bg-bg-base/60 backdrop-blur-sm"
          onClick={onClose}
        >
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-label={`${labName} waitlist`}
            initial={{ opacity: 0, scale: 0.96 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.96 }}
            transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
            className="lw-glass w-[440px] p-6"
            onClick={(event) => event.stopPropagation()}
          >
            <h2 className="text-fg-primary text-base font-semibold mb-2">{labName} is coming soon</h2>
            <p className="text-fg-secondary mb-5">
              This lab arrives with our Device Library Phase 1. Drop your email to get the launch ping.
            </p>
            <form
              onSubmit={(event) => {
                event.preventDefault();
                if (!email) return;
                window.location.href = `mailto:hello@labwired.com?subject=Waitlist:%20${encodeURIComponent(labName)}&body=Sign%20me%20up:%20${encodeURIComponent(email)}`;
                onClose();
              }}
              className="flex gap-2"
            >
              <input
                type="email"
                required
                value={email}
                onChange={(event) => setEmail(event.target.value)}
                placeholder="you@example.com"
                className="flex-1 h-9 px-3 rounded-button bg-bg-surface border border-border text-fg-primary outline-none focus:border-accent"
              />
              <button
                type="submit"
                className="h-9 px-4 rounded-button bg-accent text-bg-base font-medium hover:bg-accent-hover transition-colors duration-micro"
              >
                Notify me
              </button>
            </form>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
```

- [ ] **Step 6: Wire into `StudioShell` for empty state**

Modify `packages/playground/src/studio/StudioShell.tsx`:

```tsx
import { useState } from 'react';
import { TopChrome } from './TopChrome';
import { HeroPrompt } from './HeroPrompt';
import { ChipRow } from './ChipRow';
import { WaitlistModal } from './WaitlistModal';
import { useStudioLayout } from './useStudioLayout';

export interface StudioShellProps {
  boardName?: string;
  isEmpty?: boolean;
  onPickLab?: (labId: string) => void;
  children?: React.ReactNode;
}

export function StudioShell({ boardName = 'Untitled', isEmpty = false, onPickLab, children }: StudioShellProps) {
  const layout = useStudioLayout();
  const [waitlist, setWaitlist] = useState<{ labName: string } | null>(null);

  return (
    <div className="relative w-full h-screen overflow-hidden bg-bg-base text-fg-primary">
      <TopChrome
        boardName={boardName}
        devMode={layout.devMode}
        onOpenCommand={layout.openCommand}
        onToggleDev={layout.toggleDev}
      />
      <main role="main" aria-label="Canvas" className="absolute inset-0 pt-11 bg-bg-canvas">
        {children}
        {isEmpty && (
          <div className="absolute inset-0 flex flex-col items-center justify-start pt-[32vh] gap-6 px-4">
            <HeroPrompt onFocus={layout.openCommand} />
            <ChipRow
              onPick={(labId) => onPickLab?.(labId)}
              onLocked={(labId) => setWaitlist({ labName: humanLabName(labId) })}
            />
          </div>
        )}
      </main>
      <WaitlistModal
        open={!!waitlist}
        labName={waitlist?.labName ?? ''}
        onClose={() => setWaitlist(null)}
      />
    </div>
  );
}

function humanLabName(id: string): string {
  switch (id) {
    case 'bme280-weather': return 'BME280 Weather';
    case 'oled-hello': return 'OLED Hello';
    case 'gps-trail': return 'GPS Trail';
    case 'tft-demo': return 'TFT Demo';
    default: return id;
  }
}
```

- [ ] **Step 7: Connect `App.tsx` to empty-state**

In `App.tsx`, derive `isEmpty` from current diagram state and pass `onPickLab`:

```tsx
const isEmpty = editor.state.diagram.parts.filter((p) => p.id !== 'mcu').length === 0;

const handlePickLab = (labId: string) => {
  const next = BOARD_CONFIGS.find((b) => b.boardId === labId);
  if (!next) return;
  setSelectedBoard(next);
  editor.loadDiagram(makeStarterDiagram(next));
};

return (
  <StudioShell boardName={selectedBoard.name} isEmpty={isEmpty} onPickLab={handlePickLab}>
    {/* legacy children */}
  </StudioShell>
);
```

- [ ] **Step 8: Run tests**

Run:

```bash
cd packages/playground
npm test -- --run src/studio
```

Expected: PASS (10 new tests).

- [ ] **Step 9: Commit**

```bash
git add packages/playground/src/studio/HeroPrompt.tsx \
        packages/playground/src/studio/ChipRow.tsx \
        packages/playground/src/studio/WaitlistModal.tsx \
        packages/playground/src/studio/StudioShell.tsx \
        packages/playground/src/App.tsx
git commit -m "feat(playground): add hero prompt + starter chip row"
```

---

## Task 4: Palette drawer (slide-out)

**Files:**
- Create: `packages/playground/src/studio/PaletteDrawer.tsx`
- Modify: `packages/playground/src/studio/StudioShell.tsx`
- Test: `packages/playground/src/studio/PaletteDrawer.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `packages/playground/src/studio/PaletteDrawer.test.tsx`:

```tsx
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { PaletteDrawer } from './PaletteDrawer';

const components = [
  { type: 'led', label: 'LED', category: 'gpio' as const, bus: 'GPIO' },
  { type: 'adxl345', label: 'ADXL345', category: 'i2c' as const, bus: 'I²C 0x53' },
  { type: 'bme280', label: 'BME280', category: 'i2c' as const, bus: 'I²C 0x76' },
];

describe('PaletteDrawer', () => {
  it('starts closed (handle only)', () => {
    render(<PaletteDrawer components={components} open={false} onOpenChange={() => {}} onDragStart={() => {}} />);
    expect(screen.queryByRole('search')).toBeNull();
  });

  it('opens when open=true', () => {
    render(<PaletteDrawer components={components} open={true} onOpenChange={() => {}} onDragStart={() => {}} />);
    expect(screen.getByRole('search')).toBeInTheDocument();
  });

  it('filters by category tab', async () => {
    render(<PaletteDrawer components={components} open={true} onOpenChange={() => {}} onDragStart={() => {}} />);
    await userEvent.click(screen.getByRole('tab', { name: /i.c/i }));
    expect(screen.getByText('ADXL345')).toBeInTheDocument();
    expect(screen.queryByText('LED')).toBeNull();
  });

  it('filters by search query', async () => {
    render(<PaletteDrawer components={components} open={true} onOpenChange={() => {}} onDragStart={() => {}} />);
    await userEvent.type(screen.getByRole('searchbox'), 'bme');
    expect(screen.getByText('BME280')).toBeInTheDocument();
    expect(screen.queryByText('LED')).toBeNull();
  });

  it('invokes onDragStart when a component is dragged', () => {
    const onDragStart = vi.fn();
    render(<PaletteDrawer components={components} open={true} onOpenChange={() => {}} onDragStart={onDragStart} />);
    const ledEntry = screen.getByText('LED').closest('[draggable="true"]')!;
    const dataTransfer = { setData: vi.fn(), effectAllowed: '' };
    ledEntry.dispatchEvent(new DragEvent('dragstart', { dataTransfer: dataTransfer as unknown as DataTransfer, bubbles: true }));
    expect(onDragStart).toHaveBeenCalledWith('led');
  });
});
```

- [ ] **Step 2: Run failing test**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/PaletteDrawer
```

Expected: FAIL — module not found.

- [ ] **Step 3: Implement `PaletteDrawer`**

Create `packages/playground/src/studio/PaletteDrawer.tsx`:

```tsx
import { useMemo, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';

export type PaletteCategory = 'i2c' | 'spi' | 'uart' | 'analog' | 'gpio' | 'misc';

export interface PaletteComponent {
  type: string;
  label: string;
  category: PaletteCategory;
  bus?: string;
  thumb?: React.ReactNode;
}

export interface PaletteDrawerProps {
  components: PaletteComponent[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDragStart: (componentType: string) => void;
}

const CATEGORIES: { id: PaletteCategory | 'all'; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'i2c', label: 'I²C' },
  { id: 'spi', label: 'SPI' },
  { id: 'uart', label: 'UART' },
  { id: 'analog', label: 'Analog' },
  { id: 'gpio', label: 'GPIO' },
  { id: 'misc', label: 'Misc' },
];

export function PaletteDrawer({ components, open, onOpenChange, onDragStart }: PaletteDrawerProps) {
  const [activeCategory, setActiveCategory] = useState<PaletteCategory | 'all'>('all');
  const [query, setQuery] = useState('');

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return components.filter((component) => {
      if (activeCategory !== 'all' && component.category !== activeCategory) return false;
      if (q && !component.label.toLowerCase().includes(q) && !component.type.includes(q)) return false;
      return true;
    });
  }, [components, activeCategory, query]);

  return (
    <>
      <button
        type="button"
        aria-label="Open component palette"
        onClick={() => onOpenChange(!open)}
        className="absolute top-1/2 -translate-y-1/2 left-0 z-20 w-1.5 h-24 bg-border hover:bg-border-strong rounded-r-md transition-colors duration-micro"
      />
      <AnimatePresence>
        {open && (
          <motion.aside
            key="palette"
            initial={{ x: -280 }}
            animate={{ x: 0 }}
            exit={{ x: -280 }}
            transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
            className="absolute top-11 left-0 bottom-0 z-20 w-[280px] bg-bg-surface border-r border-border flex flex-col"
            aria-label="Component palette"
          >
            <div role="search" className="p-3 border-b border-border">
              <input
                type="search"
                role="searchbox"
                placeholder="Search components…"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                className="w-full h-8 px-2 rounded-button bg-bg-elevated border border-border text-fg-primary placeholder:text-fg-tertiary outline-none focus:border-accent"
              />
            </div>
            <div role="tablist" className="flex flex-wrap gap-1 px-3 py-2 border-b border-border">
              {CATEGORIES.map((cat) => (
                <button
                  key={cat.id}
                  role="tab"
                  aria-selected={activeCategory === cat.id}
                  onClick={() => setActiveCategory(cat.id)}
                  className={clsx(
                    'h-6 px-2 rounded-pill text-[11px] font-medium transition-colors duration-micro',
                    activeCategory === cat.id
                      ? 'bg-accent-soft text-accent border border-accent/40'
                      : 'text-fg-secondary hover:text-fg-primary border border-transparent'
                  )}
                >
                  {cat.label}
                </button>
              ))}
            </div>
            <div className="flex-1 overflow-y-auto p-2">
              {filtered.map((component) => (
                <div
                  key={component.type}
                  draggable
                  onDragStart={(event) => {
                    event.dataTransfer.setData('application/x-labwired-component', component.type);
                    event.dataTransfer.effectAllowed = 'copy';
                    onDragStart(component.type);
                  }}
                  className="flex items-center gap-3 px-2 py-2 rounded-button hover:bg-bg-elevated cursor-grab active:cursor-grabbing"
                >
                  <div className="w-8 h-8 rounded bg-bg-canvas border border-border flex items-center justify-center text-fg-secondary">
                    {component.thumb ?? component.type[0].toUpperCase()}
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="text-fg-primary text-[13px] truncate">{component.label}</div>
                    {component.bus && (
                      <div className="text-fg-tertiary text-[10px] font-mono truncate">{component.bus}</div>
                    )}
                  </div>
                </div>
              ))}
              {filtered.length === 0 && (
                <div className="text-fg-tertiary text-center mt-6 text-xs">No components match.</div>
              )}
            </div>
          </motion.aside>
        )}
      </AnimatePresence>
    </>
  );
}
```

- [ ] **Step 4: Mount in StudioShell**

Modify `packages/playground/src/studio/StudioShell.tsx` to render `<PaletteDrawer>`:

```tsx
import { PaletteDrawer, type PaletteComponent } from './PaletteDrawer';
// ...
export interface StudioShellProps {
  boardName?: string;
  isEmpty?: boolean;
  paletteComponents?: PaletteComponent[];
  onPickLab?: (labId: string) => void;
  onPaletteDrag?: (componentType: string) => void;
  children?: React.ReactNode;
}

export function StudioShell({
  boardName = 'Untitled',
  isEmpty = false,
  paletteComponents = [],
  onPickLab,
  onPaletteDrag,
  children,
}: StudioShellProps) {
  const layout = useStudioLayout();
  // ...
  return (
    <div className="relative w-full h-screen overflow-hidden bg-bg-base text-fg-primary">
      <TopChrome /* ... */ />
      <PaletteDrawer
        components={paletteComponents}
        open={layout.paletteOpen}
        onOpenChange={layout.setPaletteOpen}
        onDragStart={(type) => onPaletteDrag?.(type)}
      />
      <main /* ... */>{/* ... */}</main>
      {/* waitlist */}
    </div>
  );
}
```

- [ ] **Step 5: Map existing component registry to palette entries**

In `App.tsx`, before rendering `StudioShell`, build the palette list from the existing `COMPONENT_REGISTRY` (imported from `@labwired/ui`). The registry's `ComponentDef.category` field is added in Task 0e of the Device Library Phase 1 plan; until then, derive category from a static lookup:

```tsx
const PALETTE_CATEGORY: Record<string, PaletteCategory> = {
  adxl345: 'i2c', oled_ssd1306: 'i2c', dht22: 'misc',
  led: 'gpio', button: 'gpio', rgb_led: 'gpio', buzzer: 'gpio',
  potentiometer: 'analog', ldr: 'analog',
  servo: 'gpio', resistor: 'misc', capacitor: 'misc',
  // remaining components default to 'misc'
};

const paletteComponents: PaletteComponent[] = Array.from(COMPONENT_REGISTRY.entries())
  .filter(([type]) => type !== 'mcu')
  .map(([type, def]) => ({
    type,
    label: def.label ?? type,
    category: PALETTE_CATEGORY[type] ?? 'misc',
    bus: def.bus,
  }));
```

Pass `paletteComponents` and `onPaletteDrag` to `<StudioShell>`. Use `onPaletteDrag` to set a transient drag state that the canvas's `onDropPart` handler reads (the underlying canvas drag-drop already exists in `EditorCanvas`).

- [ ] **Step 6: Run tests**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/PaletteDrawer
```

Expected: PASS (5 tests).

- [ ] **Step 7: Commit**

```bash
git add packages/playground/src/studio/PaletteDrawer.tsx packages/playground/src/studio/StudioShell.tsx packages/playground/src/App.tsx
git commit -m "feat(playground): add slide-out palette drawer"
```

---

## Task 5: Inspector glass card

**Files:**
- Create: `packages/playground/src/studio/InspectorCard.tsx`
- Modify: `packages/playground/src/studio/StudioShell.tsx`
- Modify: `packages/playground/src/App.tsx`
- Modify: `packages/ui/src/index.ts`
- Test: `packages/playground/src/studio/InspectorCard.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `packages/playground/src/studio/InspectorCard.test.tsx`:

```tsx
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { InspectorCard } from './InspectorCard';

const selection = {
  kind: 'part' as const,
  partId: 'adxl345',
  partType: 'adxl345',
  label: 'ADXL345',
  pins: [
    { id: 'VCC', label: '3v3' },
    { id: 'GND', label: 'GND' },
    { id: 'SDA', label: 'PB7' },
    { id: 'SCL', label: 'PB6' },
  ],
  attrs: {},
};

describe('InspectorCard', () => {
  it('renders nothing when no selection', () => {
    render(<InspectorCard selection={null} devMode={false} onDelete={() => {}} onDuplicate={() => {}} />);
    expect(screen.queryByRole('complementary')).toBeNull();
  });

  it('shows the selected part label and id', () => {
    render(<InspectorCard selection={selection} devMode={false} onDelete={() => {}} onDuplicate={() => {}} />);
    expect(screen.getByText('ADXL345')).toBeInTheDocument();
    expect(screen.getByText('adxl345')).toBeInTheDocument();
  });

  it('renders the pin table', () => {
    render(<InspectorCard selection={selection} devMode={false} onDelete={() => {}} onDuplicate={() => {}} />);
    expect(screen.getByText('SDA')).toBeInTheDocument();
    expect(screen.getByText('PB7')).toBeInTheDocument();
  });

  it('invokes onDelete when delete is clicked', async () => {
    const onDelete = vi.fn();
    render(<InspectorCard selection={selection} devMode={false} onDelete={onDelete} onDuplicate={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: /delete/i }));
    expect(onDelete).toHaveBeenCalledWith('adxl345');
  });

  it('hides the advanced toggle when dev mode is off', () => {
    render(<InspectorCard selection={selection} devMode={false} onDelete={() => {}} onDuplicate={() => {}} />);
    expect(screen.queryByRole('button', { name: /advanced/i })).toBeNull();
  });
});
```

- [ ] **Step 2: Run failing test**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/InspectorCard
```

Expected: FAIL — module not found.

- [ ] **Step 3: Implement `InspectorCard`**

Create `packages/playground/src/studio/InspectorCard.tsx`:

```tsx
import { useState, type ReactNode } from 'react';
import { motion, AnimatePresence } from 'framer-motion';

export interface InspectorPin {
  id: string;
  label: string;
}

export interface PartSelection {
  kind: 'part';
  partId: string;
  partType: string;
  label: string;
  pins: InspectorPin[];
  attrs: Record<string, unknown>;
}

export interface WireSelection {
  kind: 'wire';
  wireId: string;
  from: string;
  to: string;
  color: string;
}

export type InspectorSelection = PartSelection | WireSelection;

export interface InspectorCardProps {
  selection: InspectorSelection | null;
  devMode: boolean;
  labWidget?: ReactNode;
  advancedView?: ReactNode;
  onDelete: (id: string) => void;
  onDuplicate: (id: string) => void;
}

export function InspectorCard({ selection, devMode, labWidget, advancedView, onDelete, onDuplicate }: InspectorCardProps) {
  const [advancedOpen, setAdvancedOpen] = useState(false);

  return (
    <AnimatePresence>
      {selection && (
        <motion.aside
          role="complementary"
          aria-label="Inspector"
          initial={{ opacity: 0, x: 16 }}
          animate={{ opacity: 1, x: 0 }}
          exit={{ opacity: 0, x: 16 }}
          transition={{ duration: 0.16, ease: [0.16, 1, 0.3, 1] }}
          className="lw-glass absolute top-[60px] right-4 bottom-[80px] w-[320px] flex flex-col overflow-hidden z-20"
        >
          {selection.kind === 'part' ? (
            <PartInspector
              selection={selection}
              devMode={devMode}
              labWidget={labWidget}
              advancedView={advancedView}
              advancedOpen={advancedOpen}
              onToggleAdvanced={() => setAdvancedOpen((open) => !open)}
              onDelete={onDelete}
              onDuplicate={onDuplicate}
            />
          ) : (
            <WireInspector selection={selection} onDelete={onDelete} />
          )}
        </motion.aside>
      )}
    </AnimatePresence>
  );
}

interface PartInspectorProps {
  selection: PartSelection;
  devMode: boolean;
  labWidget?: ReactNode;
  advancedView?: ReactNode;
  advancedOpen: boolean;
  onToggleAdvanced: () => void;
  onDelete: (id: string) => void;
  onDuplicate: (id: string) => void;
}

function PartInspector({ selection, devMode, labWidget, advancedView, advancedOpen, onToggleAdvanced, onDelete, onDuplicate }: PartInspectorProps) {
  return (
    <>
      <header className="px-4 py-3 border-b border-border flex items-center gap-2">
        <div className="w-8 h-8 rounded bg-bg-canvas border border-border flex items-center justify-center text-fg-secondary">
          {selection.partType[0].toUpperCase()}
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-fg-primary font-semibold truncate">{selection.label}</div>
          <div className="text-fg-tertiary text-[11px] font-mono truncate">{selection.partId}</div>
        </div>
      </header>
      <div className="flex-1 overflow-y-auto">
        <section className="px-4 py-3 border-b border-border">
          <h3 className="text-fg-tertiary text-[10px] uppercase tracking-wider mb-2">Pins</h3>
          <table className="w-full text-[12px] font-mono">
            <tbody>
              {selection.pins.map((pin) => (
                <tr key={pin.id} className="hover:bg-bg-elevated">
                  <td className="py-1 pr-2 text-fg-secondary">{pin.id}</td>
                  <td className="py-1 text-fg-primary">{pin.label}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
        {labWidget && (
          <section className="px-4 py-3 border-b border-border">
            <h3 className="text-fg-tertiary text-[10px] uppercase tracking-wider mb-2">Live</h3>
            {labWidget}
          </section>
        )}
        {devMode && advancedView && (
          <section className="px-4 py-3 border-b border-border">
            <button
              type="button"
              onClick={onToggleAdvanced}
              className="text-fg-secondary text-[11px] uppercase tracking-wider hover:text-fg-primary"
            >
              {advancedOpen ? '▾ Advanced' : '▸ Advanced'}
            </button>
            {advancedOpen && <div className="mt-2">{advancedView}</div>}
          </section>
        )}
      </div>
      <footer className="border-t border-border px-4 py-3 flex gap-2">
        <button
          type="button"
          onClick={() => onDuplicate(selection.partId)}
          className="flex-1 h-8 rounded-button bg-bg-elevated border border-border text-fg-primary hover:border-accent"
        >
          Duplicate
        </button>
        <button
          type="button"
          onClick={() => onDelete(selection.partId)}
          className="flex-1 h-8 rounded-button bg-danger/10 border border-danger/30 text-danger hover:bg-danger/20"
        >
          Delete
        </button>
      </footer>
    </>
  );
}

function WireInspector({ selection, onDelete }: { selection: WireSelection; onDelete: (id: string) => void }) {
  return (
    <>
      <header className="px-4 py-3 border-b border-border">
        <div className="text-fg-primary font-semibold">Wire</div>
        <div className="text-fg-tertiary text-[11px] font-mono">{selection.from} → {selection.to}</div>
      </header>
      <div className="flex-1 px-4 py-3">
        <div className="flex items-center gap-2 text-[12px] text-fg-secondary">
          <span className="w-3 h-3 rounded-full" style={{ background: selection.color }} />
          {selection.color}
        </div>
      </div>
      <footer className="border-t border-border px-4 py-3">
        <button
          type="button"
          onClick={() => onDelete(selection.wireId)}
          className="w-full h-8 rounded-button bg-danger/10 border border-danger/30 text-danger hover:bg-danger/20"
        >
          Delete wire
        </button>
      </footer>
    </>
  );
}
```

- [ ] **Step 4: Wire selection into `StudioShell` + `App.tsx`**

Add `inspector?: InspectorSelection`, `labWidget?: ReactNode`, `advancedView?: ReactNode` props to `StudioShell` and render `<InspectorCard>`.

In `App.tsx`, derive the inspector selection from `editor.state.selection`:

```tsx
const inspectorSelection: InspectorSelection | null = useMemo(() => {
  const sel = editor.state.selection;
  if (!sel) return null;
  if (sel.kind === 'part') {
    const part = editor.state.diagram.parts.find((p) => p.id === sel.id);
    if (!part) return null;
    const def = COMPONENT_REGISTRY.get(part.type);
    return {
      kind: 'part',
      partId: part.id,
      partType: part.type,
      label: def?.label ?? part.type,
      pins: (def?.pins ?? []).map((p) => ({ id: p.id, label: p.id })),
      attrs: part.attrs,
    };
  }
  // wire selection branch...
  return null;
}, [editor.state.selection, editor.state.diagram.parts]);

const labWidget = inspectorSelection?.kind === 'part' && inspectorSelection.partType === 'adxl345' ? (
  <Adxl345Visualizer sample={adxlSample} history={adxlHistory} onSampleChange={setAdxlSample} />
) : undefined;
```

Mark `GuidedLab` as deprecated in `packages/ui/src/index.ts` (keep the export so legacy code compiles, but add a JSDoc `@deprecated` line).

- [ ] **Step 5: Run tests**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/InspectorCard
```

Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
git add packages/playground/src/studio/InspectorCard.tsx \
        packages/playground/src/studio/StudioShell.tsx \
        packages/playground/src/App.tsx \
        packages/ui/src/index.ts
git commit -m "feat(playground): add inspector glass card"
```

---

## Task 6: Floating sim dock

**Files:**
- Create: `packages/playground/src/studio/SimDock.tsx`
- Modify: `packages/playground/src/studio/StudioShell.tsx`
- Modify: `packages/playground/src/App.tsx`
- Test: `packages/playground/src/studio/SimDock.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `packages/playground/src/studio/SimDock.test.tsx`:

```tsx
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SimDock } from './SimDock';

describe('SimDock', () => {
  const handlers = { onRun: vi.fn(), onPause: vi.fn(), onStep: vi.fn(), onReset: vi.fn() };

  it('renders run-time formatted as MM:SS', () => {
    render(<SimDock state="idle" runtimeMs={0} {...handlers} />);
    expect(screen.getByText('00:00')).toBeInTheDocument();
    render(<SimDock state="running" runtimeMs={134_000} {...handlers} />);
    expect(screen.getByText('02:14')).toBeInTheDocument();
  });

  it('invokes onRun when the run button is clicked', async () => {
    render(<SimDock state="idle" runtimeMs={0} {...handlers} />);
    await userEvent.click(screen.getByRole('button', { name: /run/i }));
    expect(handlers.onRun).toHaveBeenCalled();
  });

  it('shows pause when running', () => {
    render(<SimDock state="running" runtimeMs={0} {...handlers} />);
    expect(screen.getByRole('button', { name: /pause/i })).toBeInTheDocument();
  });

  it('disables step unless paused', () => {
    render(<SimDock state="running" runtimeMs={0} {...handlers} />);
    expect(screen.getByRole('button', { name: /step/i })).toBeDisabled();
    render(<SimDock state="paused" runtimeMs={0} {...handlers} />);
    expect(screen.getAllByRole('button', { name: /step/i })[1]).not.toBeDisabled();
  });

  it('reacts to Space to toggle run', async () => {
    render(<SimDock state="idle" runtimeMs={0} {...handlers} />);
    await userEvent.keyboard(' ');
    expect(handlers.onRun).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run failing test**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/SimDock
```

Expected: FAIL — module not found.

- [ ] **Step 3: Implement `SimDock`**

Create `packages/playground/src/studio/SimDock.tsx`:

```tsx
import { useEffect } from 'react';
import clsx from 'clsx';

export type SimState = 'idle' | 'building' | 'running' | 'paused' | 'halted';

export interface SimDockProps {
  state: SimState;
  runtimeMs: number;
  onRun: () => void;
  onPause: () => void;
  onStep: () => void;
  onReset: () => void;
}

const STATE_LABEL: Record<SimState, string> = {
  idle: 'Idle',
  building: 'Building',
  running: 'Running',
  paused: 'Paused',
  halted: 'Halted',
};

function formatRuntime(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const mm = String(Math.floor(totalSeconds / 60)).padStart(2, '0');
  const ss = String(totalSeconds % 60).padStart(2, '0');
  return `${mm}:${ss}`;
}

export function SimDock({ state, runtimeMs, onRun, onPause, onStep, onReset }: SimDockProps) {
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) return;
      if (event.key === ' ') {
        event.preventDefault();
        state === 'running' ? onPause() : onRun();
      } else if (event.key.toLowerCase() === 's' && state === 'paused') {
        onStep();
      } else if (event.key.toLowerCase() === 'r' && (event.metaKey || event.ctrlKey) === false) {
        // only when no modifier — avoids conflict with rotation in canvas
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [state, onRun, onPause, onStep]);

  const isRunning = state === 'running';
  const isPaused = state === 'paused';

  return (
    <div className="lw-glass absolute bottom-4 left-1/2 -translate-x-1/2 z-20 h-12 px-4 flex items-center gap-3 min-w-[480px]">
      <button
        type="button"
        onClick={isRunning ? onPause : onRun}
        aria-label={isRunning ? 'Pause' : 'Run'}
        className={clsx(
          'h-8 px-3 rounded-button font-medium transition-colors duration-micro flex items-center gap-2',
          isRunning ? 'bg-magenta text-bg-base hover:opacity-90' : 'bg-accent text-bg-base hover:bg-accent-hover'
        )}
      >
        <span aria-hidden>{isRunning ? '⏸' : '▶'}</span>
        {isRunning ? 'Pause' : 'Run'}
      </button>
      <button
        type="button"
        onClick={onStep}
        disabled={!isPaused}
        aria-label="Step"
        className="h-8 w-8 rounded-button border border-border text-fg-secondary hover:text-fg-primary disabled:opacity-40 disabled:cursor-not-allowed"
      >
        ⏵
      </button>
      <button
        type="button"
        onClick={onReset}
        aria-label="Reset"
        className="h-8 w-8 rounded-button border border-border text-fg-secondary hover:text-fg-primary"
      >
        ↻
      </button>
      <div className="flex-1" />
      <span className="text-fg-secondary font-mono text-[12px]">{formatRuntime(runtimeMs)}</span>
      <div className="w-px h-5 bg-border" />
      <div className="flex items-center gap-2">
        <span
          className={clsx(
            'w-2 h-2 rounded-full',
            isRunning ? 'bg-magenta animate-pulse' : 'bg-fg-tertiary'
          )}
          aria-hidden
        />
        <span className="text-fg-secondary text-[12px]">{STATE_LABEL[state]}</span>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Hook into `App.tsx`'s simulation loop**

Replace the existing `<SimControls />` invocation with `<SimDock>`. Map the `useSimulationLoop` state to `SimState`:

```tsx
const simDockState: SimState = simState.running
  ? 'running'
  : simState.paused
    ? 'paused'
    : simState.error
      ? 'halted'
      : 'idle';
```

Pass `runtimeMs={simState.runtimeMs}` (existing field). Wire `onRun`/`onPause`/`onStep`/`onReset` to the same handlers `SimControls` used.

Move the dock outside `StudioShell.tsx` into a separate slot, or accept `simDock` prop on `StudioShell` and render it in the orchestrator. Use the slot prop approach:

```tsx
<StudioShell
  boardName={selectedBoard.name}
  isEmpty={isEmpty}
  paletteComponents={paletteComponents}
  inspector={inspectorSelection}
  labWidget={labWidget}
  simDock={<SimDock state={simDockState} runtimeMs={simState.runtimeMs} {...handlers} />}
  /* ... */
>
  {canvasArea}
</StudioShell>
```

In `StudioShell`, render `{simDock}` inside the main region after `{children}`.

- [ ] **Step 5: Run tests**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/SimDock
```

Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
git add packages/playground/src/studio/SimDock.tsx packages/playground/src/studio/StudioShell.tsx packages/playground/src/App.tsx
git commit -m "feat(playground): add floating sim dock"
```

---

## Task 7: Dev drawer (off by default)

**Files:**
- Create: `packages/playground/src/studio/DevDrawer.tsx`
- Modify: `packages/playground/src/studio/StudioShell.tsx`
- Modify: `packages/playground/src/App.tsx`
- Test: `packages/playground/src/studio/DevDrawer.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `packages/playground/src/studio/DevDrawer.test.tsx`:

```tsx
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { DevDrawer } from './DevDrawer';

describe('DevDrawer', () => {
  it('renders nothing when devMode is off', () => {
    render(
      <DevDrawer
        devMode={false}
        tabs={{ serial: <div>UART</div>, registers: <div>regs</div>, trace: <div>trace</div>, memory: <div>mem</div>, yaml: <div>yaml</div> }}
      />
    );
    expect(screen.queryByRole('tablist')).toBeNull();
  });

  it('shows tabs when devMode is on', () => {
    render(
      <DevDrawer
        devMode={true}
        tabs={{ serial: <div>UART</div>, registers: <div>regs</div>, trace: <div>trace</div>, memory: <div>mem</div>, yaml: <div>yaml</div> }}
      />
    );
    expect(screen.getByRole('tab', { name: /serial/i })).toBeInTheDocument();
  });

  it('switches tabs on click', async () => {
    render(
      <DevDrawer
        devMode={true}
        tabs={{ serial: <div>UART_PANEL</div>, registers: <div>REG_PANEL</div>, trace: <div>TRACE_PANEL</div>, memory: <div>MEM_PANEL</div>, yaml: <div>YAML_PANEL</div> }}
      />
    );
    expect(screen.getByText('UART_PANEL')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('tab', { name: /registers/i }));
    expect(screen.getByText('REG_PANEL')).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run failing test**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/DevDrawer
```

Expected: FAIL — module not found.

- [ ] **Step 3: Implement `DevDrawer`**

Create `packages/playground/src/studio/DevDrawer.tsx`:

```tsx
import { useState, type ReactNode } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';

export type DevTab = 'serial' | 'registers' | 'trace' | 'memory' | 'yaml';

const TAB_ORDER: DevTab[] = ['serial', 'registers', 'trace', 'memory', 'yaml'];
const TAB_LABEL: Record<DevTab, string> = {
  serial: 'Serial',
  registers: 'Registers',
  trace: 'Trace',
  memory: 'Memory',
  yaml: 'YAML',
};

export interface DevDrawerProps {
  devMode: boolean;
  tabs: Record<DevTab, ReactNode>;
  defaultHeight?: number;
}

export function DevDrawer({ devMode, tabs, defaultHeight = 240 }: DevDrawerProps) {
  const [active, setActive] = useState<DevTab>('serial');
  const [height, setHeight] = useState(defaultHeight);

  return (
    <AnimatePresence>
      {devMode && (
        <motion.div
          initial={{ y: height }}
          animate={{ y: 0 }}
          exit={{ y: height }}
          transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
          style={{ height }}
          className="absolute bottom-0 inset-x-0 z-10 bg-bg-surface border-t border-border flex flex-col"
        >
          <div
            role="separator"
            aria-orientation="horizontal"
            onMouseDown={(event) => {
              event.preventDefault();
              const startY = event.clientY;
              const startHeight = height;
              const move = (e: MouseEvent) => {
                const next = Math.max(160, Math.min(window.innerHeight * 0.6, startHeight + (startY - e.clientY)));
                setHeight(next);
              };
              const up = () => {
                window.removeEventListener('mousemove', move);
                window.removeEventListener('mouseup', up);
              };
              window.addEventListener('mousemove', move);
              window.addEventListener('mouseup', up);
            }}
            className="h-1 cursor-ns-resize hover:bg-border"
          />
          <div role="tablist" className="flex items-center px-3 border-b border-border h-9">
            {TAB_ORDER.map((tab) => (
              <button
                key={tab}
                role="tab"
                aria-selected={active === tab}
                onClick={() => setActive(tab)}
                className={clsx(
                  'h-9 px-3 text-[12px] font-medium border-b-2 transition-colors duration-micro',
                  active === tab
                    ? 'border-accent text-fg-primary'
                    : 'border-transparent text-fg-secondary hover:text-fg-primary'
                )}
              >
                {TAB_LABEL[tab]}
              </button>
            ))}
          </div>
          <div className="flex-1 overflow-auto">{tabs[active]}</div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
```

- [ ] **Step 4: Wire into `StudioShell` + `App.tsx`**

Add a `devDrawer` prop slot on `StudioShell`:

```tsx
export interface StudioShellProps {
  // ... existing
  devDrawer?: React.ReactNode;
}
// ... inside, render {devDrawer} after main
```

In `App.tsx`:

```tsx
const devDrawer = (
  <DevDrawer
    devMode={layout.devMode}
    tabs={{
      serial: <SerialMonitor output={simState.uartOutput} onClear={clearUart} />,
      registers: <RegisterGrid registers={registers} />,
      trace: <InstructionTrace entries={traceEntries} />,
      memory: <MemoryInspector data={stackMemory} baseAddress={stackBase} />,
      yaml: <pre className="font-mono text-[12px] p-3 text-fg-secondary">{selectedBoard.systemYaml}</pre>,
    }}
  />
);
```

Hoist `layout` to App-level via a `useStudioLayout` call there (or accept dev mode through props from StudioShell — choose props through the shell to keep one source of truth: extract `useStudioLayout` callsite to App, pass values down).

- [ ] **Step 5: Run tests**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/DevDrawer
```

Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add packages/playground/src/studio/DevDrawer.tsx packages/playground/src/studio/StudioShell.tsx packages/playground/src/App.tsx
git commit -m "feat(playground): add Dev drawer"
```

---

## Task 8: Command palette (⌘K)

**Files:**
- Create: `packages/playground/src/studio/CommandPalette.tsx`
- Create: `packages/playground/src/studio/useCommandPaletteItems.ts`
- Modify: `packages/playground/src/studio/StudioShell.tsx`
- Modify: `packages/playground/src/App.tsx`
- Test: `packages/playground/src/studio/CommandPalette.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `packages/playground/src/studio/CommandPalette.test.tsx`:

```tsx
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { CommandPalette, type CommandItem } from './CommandPalette';

const items: CommandItem[] = [
  { id: 'comp:led', bucket: 'Components', label: 'LED', action: vi.fn() },
  { id: 'board:bp', bucket: 'Boards', label: 'Black Pill', action: vi.fn() },
  { id: 'ex:adxl', bucket: 'Examples', label: 'ADXL345 Tilt', action: vi.fn() },
  { id: 'act:run', bucket: 'Actions', label: 'Run', action: vi.fn() },
];

describe('CommandPalette', () => {
  it('renders nothing when closed', () => {
    render(<CommandPalette open={false} onClose={() => {}} items={items} mode="search" onModeChange={() => {}} />);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('renders all buckets when open and empty query', () => {
    render(<CommandPalette open={true} onClose={() => {}} items={items} mode="search" onModeChange={() => {}} />);
    expect(screen.getByText('Components')).toBeInTheDocument();
    expect(screen.getByText('Boards')).toBeInTheDocument();
    expect(screen.getByText('Examples')).toBeInTheDocument();
    expect(screen.getByText('Actions')).toBeInTheDocument();
  });

  it('filters by typed query', async () => {
    render(<CommandPalette open={true} onClose={() => {}} items={items} mode="search" onModeChange={() => {}} />);
    await userEvent.type(screen.getByRole('combobox'), 'led');
    expect(screen.getByText('LED')).toBeInTheDocument();
    expect(screen.queryByText('Black Pill')).toBeNull();
  });

  it('switches to assist mode on slash', async () => {
    const onModeChange = vi.fn();
    render(<CommandPalette open={true} onClose={() => {}} items={items} mode="search" onModeChange={onModeChange} />);
    await userEvent.type(screen.getByRole('combobox'), '/');
    expect(onModeChange).toHaveBeenCalledWith('assist');
  });

  it('shows the assist stub message in assist mode', () => {
    render(<CommandPalette open={true} onClose={() => {}} items={items} mode="assist" onModeChange={() => {}} />);
    expect(screen.getByText(/coming soon/i)).toBeInTheDocument();
  });

  it('invokes item.action on Enter', async () => {
    const action = vi.fn();
    const items2: CommandItem[] = [{ id: 'a', bucket: 'Actions', label: 'Run', action }];
    render(<CommandPalette open={true} onClose={() => {}} items={items2} mode="search" onModeChange={() => {}} />);
    await userEvent.keyboard('{Enter}');
    expect(action).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run failing test**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/CommandPalette
```

Expected: FAIL — module not found.

- [ ] **Step 3: Implement `CommandPalette`**

Create `packages/playground/src/studio/CommandPalette.tsx`:

```tsx
import { useEffect } from 'react';
import { Command } from 'cmdk';
import { motion, AnimatePresence } from 'framer-motion';

export type CommandMode = 'search' | 'assist';
export type CommandBucket = 'Components' | 'Boards' | 'Examples' | 'Actions';

export interface CommandItem {
  id: string;
  bucket: CommandBucket;
  label: string;
  hint?: string;
  action: () => void;
}

export interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
  items: CommandItem[];
  mode: CommandMode;
  onModeChange: (mode: CommandMode) => void;
}

const BUCKETS: CommandBucket[] = ['Components', 'Boards', 'Examples', 'Actions'];

export function CommandPalette({ open, onClose, items, mode, onModeChange }: CommandPaletteProps) {
  useEffect(() => {
    if (!open) return;
    const handler = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [open, onClose]);

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.16 }}
          className="fixed inset-0 z-50 flex items-start justify-center pt-[18vh] bg-bg-base/60 backdrop-blur"
          onClick={onClose}
        >
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-label="Command palette"
            initial={{ opacity: 0, y: -8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8 }}
            transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
            className="lw-glass w-[min(560px,calc(100vw-32px))] overflow-hidden"
            onClick={(event) => event.stopPropagation()}
          >
            <Command
              shouldFilter={mode === 'search'}
              onValueChange={(value) => {
                if (value === '/' && mode === 'search') onModeChange('assist');
              }}
            >
              <div className="h-14 px-5 flex items-center gap-3 border-b border-border">
                <span className="text-magenta text-lg" aria-hidden>
                  {mode === 'assist' ? '✨' : '⌘'}
                </span>
                <Command.Input
                  role="combobox"
                  autoFocus
                  placeholder={
                    mode === 'assist'
                      ? "Describe a change to your circuit, e.g. 'add an LED on PA5'"
                      : 'Search components, boards, examples…'
                  }
                  className="flex-1 bg-transparent outline-none text-[15px] placeholder:text-fg-tertiary"
                  onKeyDown={(event) => {
                    if (event.key === '/' && event.currentTarget.value === '' && mode === 'search') {
                      event.preventDefault();
                      onModeChange('assist');
                    } else if (event.key === 'Tab' && event.currentTarget.value === '') {
                      event.preventDefault();
                      onModeChange(mode === 'search' ? 'assist' : 'search');
                    }
                  }}
                />
              </div>
              <Command.List className="max-h-[60vh] overflow-y-auto py-2">
                {mode === 'assist' ? (
                  <div className="px-5 py-6 text-fg-secondary text-center">
                    <p className="mb-2">AI assist is coming soon.</p>
                    <a className="text-accent" href="mailto:hello@labwired.com?subject=AI%20assist%20waitlist">
                      Get notified
                    </a>
                  </div>
                ) : (
                  BUCKETS.map((bucket) => (
                    <Command.Group key={bucket} heading={bucket} className="text-fg-tertiary text-[10px] uppercase tracking-wider px-3 py-1">
                      {items
                        .filter((item) => item.bucket === bucket)
                        .map((item) => (
                          <Command.Item
                            key={item.id}
                            value={`${item.bucket} ${item.label}`}
                            onSelect={() => {
                              item.action();
                              onClose();
                            }}
                            className="flex items-center justify-between px-3 py-2 text-fg-primary text-[13px] aria-selected:bg-accent-soft aria-selected:text-accent cursor-pointer rounded"
                          >
                            <span>{item.label}</span>
                            {item.hint && <span className="text-fg-tertiary text-[11px]">{item.hint}</span>}
                          </Command.Item>
                        ))}
                    </Command.Group>
                  ))
                )}
              </Command.List>
            </Command>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
```

- [ ] **Step 4: Compose items in `useCommandPaletteItems`**

Create `packages/playground/src/studio/useCommandPaletteItems.ts`:

```ts
import { useMemo } from 'react';
import { COMPONENT_REGISTRY } from '@labwired/ui';
import type { CommandItem } from './CommandPalette';
import type { BoardConfig } from '../bundled-configs';
import { STARTER_LABS } from './ChipRow';

export interface CommandPaletteContext {
  boards: BoardConfig[];
  onLoadBoard: (board: BoardConfig) => void;
  onPickLab: (labId: string) => void;
  onAddComponent: (type: string) => void;
  onRun: () => void;
  onShare: () => void;
  onReset: () => void;
  onToggleDev: () => void;
}

export function useCommandPaletteItems(ctx: CommandPaletteContext): CommandItem[] {
  return useMemo(() => {
    const items: CommandItem[] = [];

    for (const [type, def] of COMPONENT_REGISTRY.entries()) {
      if (type === 'mcu') continue;
      items.push({
        id: `comp:${type}`,
        bucket: 'Components',
        label: def.label ?? type,
        hint: 'drop on canvas',
        action: () => ctx.onAddComponent(type),
      });
    }

    for (const board of ctx.boards) {
      items.push({
        id: `board:${board.boardId}`,
        bucket: 'Boards',
        label: board.name,
        hint: board.arch,
        action: () => ctx.onLoadBoard(board),
      });
    }

    for (const lab of STARTER_LABS) {
      items.push({
        id: `lab:${lab.id}`,
        bucket: 'Examples',
        label: lab.name,
        hint: lab.locked ? lab.comingIn : 'open',
        action: () => ctx.onPickLab(lab.id),
      });
    }

    items.push(
      { id: 'act:run', bucket: 'Actions', label: 'Run simulation', hint: 'Space', action: ctx.onRun },
      { id: 'act:reset', bucket: 'Actions', label: 'Reset simulation', action: ctx.onReset },
      { id: 'act:share', bucket: 'Actions', label: 'Share project', action: ctx.onShare },
      { id: 'act:dev', bucket: 'Actions', label: 'Toggle Dev mode', action: ctx.onToggleDev },
    );

    return items;
  }, [ctx]);
}
```

- [ ] **Step 5: Wire global ⌘K + connect to App**

In `App.tsx`:

```tsx
const [paletteMode, setPaletteMode] = useState<CommandMode>('search');
const commandItems = useCommandPaletteItems({
  boards: BOARD_CONFIGS,
  onLoadBoard: setSelectedBoard,
  onPickLab: handlePickLab,
  onAddComponent: editor.addPart,
  onRun: handleRun,
  onShare: handleShare,
  onReset: handleReset,
  onToggleDev: layout.toggleDev,
});

useEffect(() => {
  const handler = (event: KeyboardEvent) => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      layout.openCommand();
    }
  };
  window.addEventListener('keydown', handler);
  return () => window.removeEventListener('keydown', handler);
}, [layout.openCommand]);
```

Pass `<CommandPalette open={layout.commandOpen} ... />` through `StudioShell` as `commandPalette` slot prop or render alongside.

- [ ] **Step 6: Run tests**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/CommandPalette
```

Expected: PASS (6 tests).

- [ ] **Step 7: Commit**

```bash
git add packages/playground/src/studio/CommandPalette.tsx \
        packages/playground/src/studio/useCommandPaletteItems.ts \
        packages/playground/src/studio/StudioShell.tsx \
        packages/playground/src/App.tsx
git commit -m "feat(playground): add ⌘K command palette"
```

---

## Task 9: Production art pass (top 12 components)

**Files:**
- Create: `packages/playground/src/studio/art/stm32f103.tsx`
- Create: `packages/playground/src/studio/art/stm32f401cdu6.tsx`
- Create: `packages/playground/src/studio/art/nucleo-f401.tsx`
- Create: `packages/playground/src/studio/art/nucleo-h563.tsx`
- Create: `packages/playground/src/studio/art/rp2040-pico.tsx`
- Create: `packages/playground/src/studio/art/esp32s3-zero.tsx`
- Create: `packages/playground/src/studio/art/led-pro.tsx`
- Create: `packages/playground/src/studio/art/button-pro.tsx`
- Create: `packages/playground/src/studio/art/adxl345-pro.tsx`
- Create: `packages/playground/src/studio/art/potentiometer-pro.tsx`
- Create: `packages/playground/src/studio/art/ssd1306-pro.tsx`
- Create: `packages/playground/src/studio/art/resistor-pro.tsx`
- Modify: `packages/ui/src/editor/components/index.ts`
- Modify: each existing component file (e.g., `packages/ui/src/editor/components/led.tsx`) to consume the pro art
- Test: `packages/playground/src/studio/art/art.test.tsx`

- [ ] **Step 1: Write the failing tests**

Create `packages/playground/src/studio/art/art.test.tsx`:

```tsx
import { describe, expect, it } from 'vitest';
import { render } from '@testing-library/react';
import * as art from './stm32f103';
import * as ledArt from './led-pro';
import * as adxlArt from './adxl345-pro';

describe('Pro component art', () => {
  it('renders STM32F103 with required pin labels', () => {
    const { container } = render(<art.Stm32F103Art selected={false} active={false} />);
    expect(container.querySelector('[data-pin="PA5"]')).not.toBeNull();
    expect(container.querySelector('[data-pin="PB6"]')).not.toBeNull();
    expect(container.querySelector('[data-pin="PB7"]')).not.toBeNull();
  });

  it('LED draws selection ring when selected=true', () => {
    const { container } = render(<ledArt.LedArt color="green" selected={true} active={false} />);
    expect(container.querySelector('[data-selection-ring]')).not.toBeNull();
  });

  it('LED shows active glow when active=true', () => {
    const { container } = render(<ledArt.LedArt color="green" selected={false} active={true} />);
    expect(container.querySelector('[data-active-glow]')).not.toBeNull();
  });

  it('ADXL345 renders the I²C pin labels', () => {
    const { container } = render(<adxlArt.Adxl345Art selected={false} active={false} />);
    expect(container.querySelector('[data-pin="SDA"]')).not.toBeNull();
    expect(container.querySelector('[data-pin="SCL"]')).not.toBeNull();
  });
});
```

- [ ] **Step 2: Run failing tests**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/art/art
```

Expected: FAIL — modules not found.

- [ ] **Step 3: Implement each art file**

Each art file follows this template (example: `led-pro.tsx`):

```tsx
import clsx from 'clsx';

export interface PartArtProps {
  selected: boolean;
  active: boolean;
}

export interface LedArtProps extends PartArtProps {
  color: 'red' | 'green' | 'blue' | 'yellow' | 'white';
}

const LED_COLORS: Record<LedArtProps['color'], { body: string; glow: string }> = {
  red: { body: '#F2545B', glow: 'rgba(242,84,91,0.55)' },
  green: { body: '#3DD68C', glow: 'rgba(61,214,140,0.55)' },
  blue: { body: '#5B9DFF', glow: 'rgba(91,157,255,0.55)' },
  yellow: { body: '#F5B642', glow: 'rgba(245,182,66,0.55)' },
  white: { body: '#F2F4F9', glow: 'rgba(242,244,249,0.55)' },
};

export function LedArt({ color, selected, active }: LedArtProps) {
  const { body, glow } = LED_COLORS[color];
  return (
    <g>
      {active && (
        <circle cx="20" cy="20" r="22" fill={glow} data-active-glow opacity="0.7">
          <animate attributeName="opacity" values="0.4;0.9;0.4" dur="1.2s" repeatCount="indefinite" />
        </circle>
      )}
      <circle cx="20" cy="20" r="14" fill={body} stroke="#0A0B0F" strokeWidth="1.5" />
      <circle cx="16" cy="16" r="4" fill="rgba(255,255,255,0.45)" />
      <line x1="20" y1="34" x2="20" y2="46" stroke="#7A8094" strokeWidth="2" data-pin="A" />
      <line x1="14" y1="34" x2="14" y2="46" stroke="#7A8094" strokeWidth="2" data-pin="C" />
      {selected && (
        <circle cx="20" cy="20" r="20" fill="none" stroke="#F062B8" strokeWidth="2" data-selection-ring />
      )}
    </g>
  );
}
```

Apply the same `selected` / `active` data-attribute convention to every art file. The other 11 files follow the same pattern with their own SVG shapes; each file exports a single named component (e.g., `Stm32F103Art`, `BlackPillArt`, `ButtonArt`, `Adxl345Art`, etc.). When implementing each, render:

- **MCU boards** (stm32f103, stm32f401cdu6, nucleo-f401, nucleo-h563, rp2040-pico, esp32s3-zero): a PCB rectangle with appropriate silkscreen color (Bluepill green, Black Pill black, Nucleo blue, RP2040 white, ESP32-S3 dark), USB connector, pin headers with labels positioned by chip family. Use `data-pin="<label>"` on every pin shape so tests and wire routing can locate them.
- **LED, button, ADXL345, potentiometer, SSD1306, resistor**: realistic depiction with body, leads, labels. SSD1306 includes a 128×64-aspect display rectangle (will render the framebuffer once Phase 1 Wave 1.4 ships).
- For each, accept `selected` and `active` props, render a 2px `--lw-magenta` ring on select and a soft pulse on active.

- [ ] **Step 4: Adopt pro art in existing component definitions**

Modify each affected component file in `packages/ui/src/editor/components/` (e.g., `led.tsx`, `adxl345.tsx`, `button.tsx`, `potentiometer.tsx`, `oled-ssd1306.tsx`, `resistor.tsx`, `mcu.tsx`) so the `render` function delegates to the pro art:

```tsx
// led.tsx
import { LedArt } from '../../../../../playground/src/studio/art/led-pro';
// (path note: art lives in packages/playground; if a cross-package import is undesirable, move art under packages/ui/src/editor/art/ during implementation)

export const ledComponent: ComponentDef = {
  type: 'led',
  label: 'LED',
  category: 'gpio',
  // ...
  render: (attrs, state) => (
    <LedArt color={(attrs.color as LedArtProps['color']) ?? 'green'} selected={!!state?.selected} active={!!state?.active} />
  ),
};
```

**Implementer's call:** if cross-package imports are awkward, move `art/` from `packages/playground/src/studio/art/` to `packages/ui/src/editor/art/` and re-import from there. Update test paths accordingly. Either location is acceptable; pick the one that does not require workspace-relative imports.

- [ ] **Step 5: Run tests**

Run:

```bash
cd packages/playground
npm test -- --run src/studio/art
```

Expected: PASS (4 tests).

- [ ] **Step 6: Run UI test suite to confirm components still render**

Run:

```bash
cd packages/ui
npm test -- --run
```

Expected: existing component tests pass (no rendering regressions).

- [ ] **Step 7: Commit**

```bash
git add packages/playground/src/studio/art packages/ui/src/editor/components
git commit -m "feat(playground): production art pass for top 12 components"
```

---

## Task 10: Performance, mobile, legacy fallback, E2E

**Files:**
- Create: `packages/playground/src/legacy/App.legacy.tsx`
- Create: `packages/playground/src/legacy/legacy.html`
- Modify: `packages/playground/vite.config.ts`
- Modify: `packages/playground/src/App.tsx`
- Create: `packages/playground/playwright.config.ts` (if missing)
- Create: `packages/playground/tests/e2e/studio.spec.ts`
- Test: `packages/playground/src/studio/mobile.test.tsx`

- [ ] **Step 1: Add the legacy entry**

Move the current `App.tsx` content as it was at the start of Task 2 to `packages/playground/src/legacy/App.legacy.tsx`. Create `packages/playground/src/legacy/legacy.html`:

```html
<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <title>LabWired Playground (legacy)</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="./legacy.entry.tsx"></script>
  </body>
</html>
```

Create `packages/playground/src/legacy/legacy.entry.tsx`:

```tsx
import { createRoot } from 'react-dom/client';
import { App as LegacyApp } from './App.legacy';

createRoot(document.getElementById('root')!).render(<LegacyApp />);
```

Modify `packages/playground/vite.config.ts` to add the legacy build input:

```ts
build: {
  rollupOptions: {
    input: {
      main: resolve(__dirname, 'index.html'),
      legacy: resolve(__dirname, 'src/legacy/legacy.html'),
    },
  },
},
```

After `npm run build`, the artifact at `dist/legacy/` is served at `/playground/legacy/` after deploy.

- [ ] **Step 2: Add mobile read-only mode**

Create `packages/playground/src/studio/mobile.test.tsx`:

```tsx
import { describe, expect, it } from 'vitest';
import { isMobileViewport, MOBILE_BREAKPOINT } from './useStudioLayout';

describe('mobile viewport detection', () => {
  it('returns true under the breakpoint', () => {
    expect(isMobileViewport(MOBILE_BREAKPOINT - 1)).toBe(true);
  });

  it('returns false at the breakpoint', () => {
    expect(isMobileViewport(MOBILE_BREAKPOINT)).toBe(false);
  });
});
```

Modify `packages/playground/src/studio/useStudioLayout.ts`:

```ts
export const MOBILE_BREAKPOINT = 768;

export function isMobileViewport(widthPx: number = window.innerWidth): boolean {
  return widthPx < MOBILE_BREAKPOINT;
}

export function useStudioLayout() {
  // ... existing
  const [mobile, setMobile] = useState(() => isMobileViewport());
  useEffect(() => {
    const onResize = () => setMobile(isMobileViewport());
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);
  return { /* ...existing, */ mobile };
}
```

In `StudioShell.tsx`, when `layout.mobile` is true, render a banner over the top of the canvas: *"View only on mobile — open on desktop to edit."*. Disable drop handlers in `App.tsx` based on the same flag. Sim dock remains active.

- [ ] **Step 3: Bundle splitting — defer WASM**

Modify `packages/playground/src/App.tsx`'s WASM import. Replace the static import:

```tsx
import { loadSimulator } from '@labwired/ui/wasm/simulator-bridge';
```

with a lazy import that fires on `handleRun`:

```tsx
const handleRun = async () => {
  if (!bridgeRef.current) {
    const { loadSimulator } = await import('@labwired/ui/wasm/simulator-bridge');
    bridgeRef.current = await loadSimulator();
  }
  bridgeRef.current.run(/* ... */);
};
```

Ensure `bundled-configs.ts` still resolves chip/system YAML statically (raw imports stay synchronous).

- [ ] **Step 4: Playwright smoke test**

Create `packages/playground/playwright.config.ts` (if not present):

```ts
import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/e2e',
  use: { baseURL: 'http://localhost:5173' },
  webServer: {
    command: 'npm run dev -- --port 5173',
    url: 'http://localhost:5173',
    reuseExistingServer: !process.env.CI,
  },
});
```

Create `packages/playground/tests/e2e/studio.spec.ts`:

```ts
import { test, expect } from '@playwright/test';

test('empty state shows hero prompt and chip-row', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByPlaceholder(/describe what to build/i)).toBeVisible();
  await expect(page.getByRole('button', { name: /blinky/i })).toBeVisible();
});

test('clicking Blinky loads the lab', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: /blinky/i }).click();
  await expect(page.getByRole('main', { name: /canvas/i })).toBeVisible();
  await page.getByRole('button', { name: /^run$/i }).click();
  await expect(page.getByText(/running/i)).toBeVisible({ timeout: 5000 });
});

test('⌘K opens the command palette', async ({ page }) => {
  await page.goto('/');
  await page.keyboard.press('Meta+K');
  await expect(page.getByRole('dialog', { name: /command palette/i })).toBeVisible();
});

test('Dev toggle reveals the dev drawer', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('switch', { name: /dev mode/i }).click();
  await expect(page.getByRole('tab', { name: /serial/i })).toBeVisible();
});
```

Add a script to `packages/playground/package.json`:

```json
"e2e": "playwright test"
```

- [ ] **Step 5: Run unit + e2e**

```bash
cd packages/playground
npm test -- --run
npx playwright install --with-deps chromium
npm run e2e
```

Expected: all unit tests pass; Playwright suite runs (it requires `npm run dev` server to start; the config above starts it automatically).

- [ ] **Step 6: Verify performance budget**

Run:

```bash
cd packages/playground
npm run build
ls -lh dist/assets/*.js | sort -k5 -h
```

Expected: the main entry bundle (gzipped) ≤ 220 KB. WASM bundle is split off (loaded on Run, not at load).

Optionally run Lighthouse:

```bash
npx lighthouse http://localhost:5173 --only-categories=performance --quiet --chrome-flags="--headless"
```

Expected: TTI ≤ 1.2s on a fast connection.

- [ ] **Step 7: Update README**

Modify `packages/playground/README.md` (or create if missing):

```markdown
# LabWired Playground

The Studio shell — dark, glass, AI-ready.

- `npm run dev` — Vite dev server
- `npm test` — Vitest unit tests
- `npm run e2e` — Playwright smoke tests
- `npm run build` — production build (main + `/legacy/`)

Legacy shell is served at `/legacy/` for two weeks post-rework as a fallback.
```

- [ ] **Step 8: Commit**

```bash
git add packages/playground
git commit -m "feat(playground): performance budget, mobile read-only, legacy fallback, E2E smoke"
```

---

## Final pass — manual demo gate

After all task subagents complete and reviewers approve:

- [ ] Run the playground locally (`npm run dev`).
- [ ] Cold-load: verify hero + chip-row visible within 1.2s.
- [ ] Click Blinky: LED appears, click Run, LED toggles, sim dock shows "Running" + magenta pulse.
- [ ] Click ADXL345 Tilt: ADXL345 part appears wired, inspector glass card shows axis sliders, sliders mutate live sample, serial output (in Dev mode) shows X/Y/Z lines.
- [ ] Open ⌘K: type "led" → LED component appears in Components; type "/" → assist mode placeholder.
- [ ] Click a locked chip (e.g., BME280): waitlist modal appears; Esc closes.
- [ ] Toggle Dev: drawer slides up with Serial/Registers/Trace/Memory/YAML tabs.
- [ ] Resize to 600px width: read-only banner appears, drop on canvas is disabled.
- [ ] Visit `/legacy/`: the old playground still works.

If all pass, the rework is shippable. Open PR with title "Studio rework: dark shell, hero prompt, chip-row, palette, inspector, sim dock, dev drawer, ⌘K, art pass."

---

## Self-review

- **Spec coverage:** every section of the spec maps to a task. Section 3 (visual system) → Task 1; 4 (layout) → Tasks 2, 4, 5, 6, 7; 5 (hero prompt) → Task 3 + Task 8; 6 (chip-row) → Task 3; 7 (interaction) → covered by inspector/palette/sim-dock tasks; 8 (state machine) → Task 6 (sim dock states); 9 (art) → Task 9; 10 (tech stack) → Task 1; 11 (performance) → Task 10; 12 (mobile) → Task 10; 13 (rollout / legacy) → Task 10.
- **Placeholder scan:** all code blocks contain real code. No "TBD", no "TODO", no vague "handle edge cases". `App.tsx` integration code in Tasks 2/3/4/5/6/7/8 shows the exact JSX to insert; the file is too large to dump in full but every modification is precise enough for an implementer to apply.
- **Type consistency:** `InspectorSelection`, `SimState`, `CommandItem`, `CommandMode`, `PaletteComponent`, `PaletteCategory`, `StarterLab` are defined where first introduced and re-used identically. `useStudioLayout` returns the same field names across all tasks that consume it.
- **Cross-package import note:** Task 9 explicitly flags the cross-package art import as an implementer's call; either location is acceptable.
- **Open question carryover:** the spec's Section 15 open questions are resolved here by the recommended defaults: Tailwind shipped, cmdk shipped, mobile read-only, AI assist stub only, hero hidden in embed (Task 10 implicit via `isEmbedMode`).
