// @vitest-environment jsdom
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
