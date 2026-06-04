// Guards the touch run-mode behavior added to EditorCanvas for MobileRunView,
// plus a desktop regression check that edit mode still selects on click.
import { describe, it, expect, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/react';
import { EditorCanvas, type EditorState, type Diagram } from '@labwired/ui';

function makeState(): EditorState {
  const diagram: Diagram = {
    version: 1,
    board: 'esp32',
    parts: [{ id: 'btn1', type: 'button', x: 100, y: 100, rotate: 0, attrs: {} }],
    wires: [],
  };
  return { diagram, selectedIds: new Set(), wireInProgress: null, undoStack: [], redoStack: [] };
}

function handlers() {
  return {
    onMovePart: vi.fn(),
    onSelect: vi.fn(),
    onStartWire: vi.fn(),
    onCompleteWire: vi.fn(),
    onCancelWire: vi.fn(),
    onDeleteWire: vi.fn(),
    onButtonToggle: vi.fn(),
  };
}

// The part group is rendered as <g transform="translate(x, y)">.
const partSelector = 'g[transform="translate(100, 100)"]';

describe('EditorCanvas run mode (touch)', () => {
  it('tapping a button presses then releases it, without authoring', () => {
    const h = handlers();
    const { container } = render(
      <EditorCanvas state={makeState()} interactionMode="run" {...h} />,
    );
    const part = container.querySelector(partSelector)!;
    expect(part).toBeTruthy();

    fireEvent.pointerDown(part, { pointerId: 1, clientX: 120, clientY: 120, button: 0 });
    fireEvent.pointerUp(part, { pointerId: 1, clientX: 120, clientY: 120, button: 0 });

    expect(h.onButtonToggle).toHaveBeenCalledWith('btn1', true);
    expect(h.onButtonToggle).toHaveBeenCalledWith('btn1', false);
    // No editing happens on a phone.
    expect(h.onMovePart).not.toHaveBeenCalled();
    expect(h.onStartWire).not.toHaveBeenCalled();
    expect(h.onSelect).not.toHaveBeenCalled();
  });
});

describe('EditorCanvas edit mode (desktop regression)', () => {
  it('clicking a part selects it and does not toggle it', () => {
    const h = handlers();
    const { container } = render(<EditorCanvas state={makeState()} {...h} />);
    const part = container.querySelector(partSelector)!;

    fireEvent.pointerDown(part, { pointerId: 1, clientX: 120, clientY: 120, button: 0 });
    fireEvent.pointerUp(part, { pointerId: 1, clientX: 120, clientY: 120, button: 0 });

    expect(h.onSelect).toHaveBeenCalledWith('btn1', false);
    expect(h.onButtonToggle).not.toHaveBeenCalled();
  });
});
