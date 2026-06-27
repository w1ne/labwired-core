import { act, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ChipWindow } from './ChipWindow';

describe('ChipWindow', () => {
  it('moves when dragged with a touch pointer', () => {
    const onClose = vi.fn();
    const onFocus = vi.fn();

    render(
      <ChipWindow
        title="ESP32"
        initial={{ x: 20, y: 30 }}
        width={300}
        height={180}
        onClose={onClose}
        onFocus={onFocus}
      >
        <div>serial</div>
      </ChipWindow>,
    );

    const dialog = screen.getByRole('dialog', { name: 'Chip serial window' });
    const handle = dialog.querySelector('[data-chip-window-drag-handle]');
    expect(handle).not.toBeNull();

    act(() => {
      handle?.dispatchEvent(new PointerEvent('pointerdown', {
        bubbles: true,
        pointerId: 1,
        pointerType: 'touch',
        clientX: 40,
        clientY: 50,
      }));
      window.dispatchEvent(new PointerEvent('pointermove', {
        bubbles: true,
        pointerId: 1,
        pointerType: 'touch',
        clientX: 140,
        clientY: 130,
      }));
      window.dispatchEvent(new PointerEvent('pointerup', {
        bubbles: true,
        pointerId: 1,
        pointerType: 'touch',
        clientX: 140,
        clientY: 130,
      }));
    });

    expect(onFocus).toHaveBeenCalled();
    expect(dialog).toHaveStyle({ left: '120px', top: '110px' });
  });
});
