// @vitest-environment jsdom
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
