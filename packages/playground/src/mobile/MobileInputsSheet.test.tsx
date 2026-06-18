// Guards the tab-visibility logic of the mobile/embed bottom sheet: a tool tab
// shows only when the lab actually EXERCISES it. Serial is always offered;
// Inputs / Logic / IO-Link appear only when the diagram wires a matching part;
// BLE appears only once real frames hit the virtual air (so a radioless board
// never shows an empty tracer).
import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import type { Diagram, Part, SimulatorBridge } from '@labwired/ui';
import { MobileInputsSheet } from './MobileInputsSheet';

function diagram(parts: Part[]): Diagram {
  return { version: 1, board: 'nrf52840', parts, wires: [] };
}

function part(over: Partial<Part> & { type: string }): Part {
  return { id: over.type, x: 0, y: 0, rotate: 0, attrs: {}, ...over };
}

/** A bridge whose virtual air already carries `frames` BLE frames. */
function bridgeWithAir(frames = 1): SimulatorBridge {
  return {
    airTraceSnapshot: () => Array.from({ length: frames }, () => ({} as never)),
  } as unknown as SimulatorBridge;
}

function renderSheet(
  d: Diagram,
  opts: { bridge?: SimulatorBridge | null; running?: boolean } = {},
) {
  return render(
    <MobileInputsSheet
      diagram={d}
      boardIoStates={{}}
      uartOutput=""
      onUpdateAttr={vi.fn()}
      ntcTemperatures={{}}
      onNtcChange={vi.fn()}
      onAnalogChange={vi.fn()}
      bridge={opts.bridge ?? null}
      running={opts.running ?? false}
      onPartAttrChange={vi.fn()}
    />,
  );
}

function tabNames(): string[] {
  return screen
    .getAllByRole('button')
    .map((b) => b.textContent?.trim() ?? '')
    .filter((t) => ['Inputs', 'Serial', 'BLE', 'Logic', 'IO-Link'].includes(t));
}

describe('MobileInputsSheet tabs', () => {
  it('always offers Serial, and hides BLE when there is no air traffic', () => {
    renderSheet(diagram([part({ type: 'led' })]));
    const tabs = tabNames();
    expect(tabs).toContain('Serial');
    expect(tabs).not.toContain('BLE');
    expect(tabs).not.toContain('Inputs');
    expect(tabs).not.toContain('Logic');
    expect(tabs).not.toContain('IO-Link');
  });

  it('reveals the BLE tab once frames are on the virtual air', () => {
    renderSheet(diagram([part({ type: 'led' })]), { bridge: bridgeWithAir(), running: true });
    expect(tabNames()).toContain('BLE');
  });

  it('shows the Inputs tab when an adjustable input part is present', () => {
    renderSheet(diagram([part({ type: 'ultrasonic' })]));
    expect(tabNames()).toContain('Inputs');
  });

  it('shows the Logic tab only when a logic-analyzer part is present', () => {
    renderSheet(diagram([part({ type: 'logic-analyzer' })]));
    expect(tabNames()).toContain('Logic');
  });

  it('shows the IO-Link tab only when an iolink-master part is present', () => {
    renderSheet(diagram([part({ type: 'iolink-master' })]));
    expect(tabNames()).toContain('IO-Link');
  });
});
