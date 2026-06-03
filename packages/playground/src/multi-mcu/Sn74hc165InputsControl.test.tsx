import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Sn74hc165InputsControl } from './Sn74hc165InputsControl';

describe('Sn74hc165InputsControl', () => {
  it('shows the input byte and toggles individual channels', async () => {
    const user = userEvent.setup();
    const onChannelChange = vi.fn();

    render(<Sn74hc165InputsControl value={0xa5} onChannelChange={onChannelChange} />);

    expect(screen.getByText('0xA5')).toBeInTheDocument();
    expect(screen.getByText('1010 0101')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'D2 HI' }));
    await user.click(screen.getByRole('button', { name: 'D0 HI' }));

    expect(onChannelChange).toHaveBeenCalledWith(2, false);
    expect(onChannelChange).toHaveBeenCalledWith(0, false);
  });

  it('can set all inputs low or high as a byte', async () => {
    const user = userEvent.setup();
    const onByteChange = vi.fn();

    render(<Sn74hc165InputsControl value={0xa5} onChannelChange={() => {}} onByteChange={onByteChange} />);

    await user.click(screen.getByRole('button', { name: '00' }));
    await user.click(screen.getByRole('button', { name: 'FF' }));

    expect(onByteChange).toHaveBeenCalledWith(0);
    expect(onByteChange).toHaveBeenCalledWith(255);
  });
});
