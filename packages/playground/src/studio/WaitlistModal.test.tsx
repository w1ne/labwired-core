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
