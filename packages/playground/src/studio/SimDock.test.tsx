import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SimDock } from './SimDock';

describe('SimDock', () => {
  const baseHandlers = { onRun: vi.fn(), onPause: vi.fn(), onStep: vi.fn(), onReset: vi.fn() };

  beforeEach(() => {
    baseHandlers.onRun.mockClear();
    baseHandlers.onPause.mockClear();
    baseHandlers.onStep.mockClear();
    baseHandlers.onReset.mockClear();
  });

  it('renders run-time formatted as MM:SS for 0ms', () => {
    render(<SimDock state="idle" runtimeMs={0} {...baseHandlers} />);
    expect(screen.getByText('00:00')).toBeInTheDocument();
  });

  it('renders run-time formatted as MM:SS for 134_000ms', () => {
    render(<SimDock state="running" runtimeMs={134_000} {...baseHandlers} />);
    expect(screen.getByText('02:14')).toBeInTheDocument();
  });

  it('invokes onRun when the run button is clicked from idle', async () => {
    render(<SimDock state="idle" runtimeMs={0} {...baseHandlers} />);
    await userEvent.click(screen.getByRole('button', { name: /^run$/i }));
    expect(baseHandlers.onRun).toHaveBeenCalled();
  });

  it('shows pause button when running', () => {
    render(<SimDock state="running" runtimeMs={0} {...baseHandlers} />);
    expect(screen.getByRole('button', { name: /^pause$/i })).toBeInTheDocument();
  });

  it('disables step unless paused', () => {
    const { rerender } = render(<SimDock state="running" runtimeMs={0} {...baseHandlers} />);
    expect(screen.getByRole('button', { name: /^step$/i })).toBeDisabled();
    rerender(<SimDock state="paused" runtimeMs={0} {...baseHandlers} />);
    expect(screen.getByRole('button', { name: /^step$/i })).not.toBeDisabled();
  });

  it('reacts to Space to toggle run from idle', async () => {
    render(<SimDock state="idle" runtimeMs={0} {...baseHandlers} />);
    await userEvent.keyboard(' ');
    expect(baseHandlers.onRun).toHaveBeenCalled();
  });

  it('reacts to Space to pause when running', async () => {
    render(<SimDock state="running" runtimeMs={0} {...baseHandlers} />);
    await userEvent.keyboard(' ');
    expect(baseHandlers.onPause).toHaveBeenCalled();
  });

  it('reacts to S to step when paused', async () => {
    render(<SimDock state="paused" runtimeMs={0} {...baseHandlers} />);
    await userEvent.keyboard('s');
    expect(baseHandlers.onStep).toHaveBeenCalled();
  });

  it('renders status label matching state', () => {
    const { rerender } = render(<SimDock state="building" runtimeMs={0} {...baseHandlers} />);
    expect(screen.getByText('Building')).toBeInTheDocument();
    rerender(<SimDock state="halted" runtimeMs={0} {...baseHandlers} />);
    expect(screen.getByText('Halted')).toBeInTheDocument();
  });
});
