import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ChipRow, STARTER_LABS } from './ChipRow';

describe('ChipRow', () => {
  it('renders every starter lab', () => {
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
});
