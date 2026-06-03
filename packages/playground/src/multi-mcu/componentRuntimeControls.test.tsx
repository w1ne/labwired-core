import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { Part } from '@labwired/ui';
import { renderComponentRuntimeControl } from './componentRuntimeControls';

describe('component runtime controls', () => {
  const part: Part = {
    id: 'di_shifter',
    type: 'sn74hc165',
    x: 0,
    y: 0,
    rotate: 0,
    attrs: { inputs: '165' },
  };

  it('renders 74HC165 input controls from diagram attributes without a live bridge', async () => {
    const user = userEvent.setup();
    const updateAttrs = vi.fn();

    render(
      <>
        {renderComponentRuntimeControl({
          part,
          bridge: null,
          updateAttrs,
        })}
      </>,
    );

    expect(screen.getByText('0xA5')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'D0 HI' }));

    expect(updateAttrs).toHaveBeenCalledWith('di_shifter', { inputs: '164' });
  });

  it('syncs 74HC165 input controls into a live bridge when present', async () => {
    const user = userEvent.setup();
    const updateAttrs = vi.fn();
    const bridge = {
      getSn74hc165Inputs: () => 0,
      setSn74hc165Inputs: vi.fn(),
    };

    render(
      <>
        {renderComponentRuntimeControl({
          part,
          bridge,
          updateAttrs,
        })}
      </>,
    );

    expect(screen.getByText('0x00')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'D2 LO' }));

    expect(updateAttrs).toHaveBeenCalledWith('di_shifter', { inputs: '4' });
    expect(bridge.setSn74hc165Inputs).toHaveBeenCalledWith(4);
  });
});
