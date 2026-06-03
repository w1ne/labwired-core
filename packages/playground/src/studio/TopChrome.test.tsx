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

  it('opens the visible tools menu from the top navigation', async () => {
    render(
      <TopChrome
        boardName="Untitled"
        onOpenCommand={() => {}}
        tools={[
          {
            id: 'iolink-analyzer',
            label: 'IO-Link Analyzer',
            active: false,
            onToggle: () => {},
          },
        ]}
      />,
    );

    expect(screen.queryByText('IO-Link Analyzer')).not.toBeInTheDocument();

    const toolsButton = screen.getByRole('button', { name: 'Tools' });
    expect(toolsButton.querySelector('svg')).toBeNull();

    await userEvent.click(toolsButton);

    expect(screen.getByText('IO-Link Analyzer')).toBeInTheDocument();
  });
});
