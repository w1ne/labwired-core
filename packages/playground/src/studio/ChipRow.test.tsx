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
