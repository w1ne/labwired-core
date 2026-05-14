import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { CommandPalette, type CommandItem } from './CommandPalette';

const items: CommandItem[] = [
  { id: 'comp:led', bucket: 'Components', label: 'LED', action: vi.fn() },
  { id: 'board:bp', bucket: 'Boards', label: 'Black Pill', action: vi.fn() },
  { id: 'ex:adxl', bucket: 'Examples', label: 'ADXL345 Tilt', action: vi.fn() },
  { id: 'act:run', bucket: 'Actions', label: 'Run', action: vi.fn() },
];

describe('CommandPalette', () => {
  it('renders nothing when closed', () => {
    render(<CommandPalette open={false} onClose={() => {}} items={items} mode="search" onModeChange={() => {}} />);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('renders all four bucket headings when open and query empty', () => {
    render(<CommandPalette open={true} onClose={() => {}} items={items} mode="search" onModeChange={() => {}} />);
    expect(screen.getByText('Components')).toBeInTheDocument();
    expect(screen.getByText('Boards')).toBeInTheDocument();
    expect(screen.getByText('Examples')).toBeInTheDocument();
    expect(screen.getByText('Actions')).toBeInTheDocument();
  });

  it('filters by typed query', async () => {
    render(<CommandPalette open={true} onClose={() => {}} items={items} mode="search" onModeChange={() => {}} />);
    await userEvent.type(screen.getByRole('combobox'), 'LED');
    expect(screen.getByText('LED')).toBeInTheDocument();
    expect(screen.queryByText('Black Pill')).toBeNull();
  });

  it('switches to assist mode when slash is typed in empty input', async () => {
    const onModeChange = vi.fn();
    render(<CommandPalette open={true} onClose={() => {}} items={items} mode="search" onModeChange={onModeChange} />);
    await userEvent.type(screen.getByRole('combobox'), '/');
    expect(onModeChange).toHaveBeenCalledWith('assist');
  });

  it('shows the assist stub message in assist mode', () => {
    render(<CommandPalette open={true} onClose={() => {}} items={items} mode="assist" onModeChange={() => {}} />);
    expect(screen.getByText(/coming soon/i)).toBeInTheDocument();
  });

  it('closes on Escape', async () => {
    const onClose = vi.fn();
    render(<CommandPalette open={true} onClose={onClose} items={items} mode="search" onModeChange={() => {}} />);
    await userEvent.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalled();
  });
});
