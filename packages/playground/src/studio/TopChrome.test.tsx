import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { TopChrome } from './TopChrome';

describe('TopChrome', () => {
  it('renders the LabWired brand link', () => {
    render(<TopChrome boardName="Untitled" onOpenCommand={() => {}} />);
    expect(screen.getByRole('link', { name: /labwired/i })).toBeInTheDocument();
  });

  it('shows the current board name in the breadcrumb', () => {
    render(<TopChrome boardName="STM32F103 Blinky" onOpenCommand={() => {}} />);
    expect(screen.getByText('STM32F103 Blinky')).toBeInTheDocument();
  });

  it('opens the command palette when the search affordance is clicked', async () => {
    const onOpenCommand = vi.fn();
    render(<TopChrome boardName="Untitled" onOpenCommand={onOpenCommand} />);
    await userEvent.click(screen.getByRole('button', { name: /search components/i }));
    expect(onOpenCommand).toHaveBeenCalled();
  });
});
