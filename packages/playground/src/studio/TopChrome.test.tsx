import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { TopChrome } from './TopChrome';

describe('TopChrome', () => {
  it('renders the LabWired brand link', () => {
    render(<TopChrome boardName="Untitled" onOpenCommand={() => {}} devMode={false} onToggleDev={() => {}} />);
    expect(screen.getByRole('link', { name: /labwired/i })).toBeInTheDocument();
  });

  it('shows the current board name in the breadcrumb', () => {
    render(<TopChrome boardName="STM32F103 Blinky" onOpenCommand={() => {}} devMode={false} onToggleDev={() => {}} />);
    expect(screen.getByText('STM32F103 Blinky')).toBeInTheDocument();
  });

  it('opens the command palette when the search affordance is clicked', async () => {
    const onOpenCommand = vi.fn();
    render(<TopChrome boardName="Untitled" onOpenCommand={onOpenCommand} devMode={false} onToggleDev={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: /open command palette/i }));
    expect(onOpenCommand).toHaveBeenCalled();
  });

  it('toggles dev mode when the Dev pill is clicked', async () => {
    const onToggleDev = vi.fn();
    render(<TopChrome boardName="Untitled" onOpenCommand={() => {}} devMode={false} onToggleDev={onToggleDev} />);
    await userEvent.click(screen.getByRole('switch', { name: /dev mode/i }));
    expect(onToggleDev).toHaveBeenCalled();
  });
});
