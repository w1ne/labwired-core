import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { DevDrawer } from './DevDrawer';

describe('DevDrawer', () => {
  const tabs = {
    serial: <div>UART_PANEL</div>,
    registers: <div>REG_PANEL</div>,
    trace: <div>TRACE_PANEL</div>,
    memory: <div>MEM_PANEL</div>,
    source: <div>SOURCE_PANEL</div>,
    yaml: <div>YAML_PANEL</div>,
  };

  it('renders nothing when devMode is off', () => {
    render(<DevDrawer devMode={false} tabs={tabs} />);
    expect(screen.queryByRole('tablist')).toBeNull();
  });

  it('shows tabs when devMode is on', () => {
    render(<DevDrawer devMode={true} tabs={tabs} />);
    expect(screen.getByRole('tab', { name: /serial/i })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /registers/i })).toBeInTheDocument();
  });

  it('switches tabs on click', async () => {
    render(<DevDrawer devMode={true} tabs={tabs} />);
    expect(screen.getByText('UART_PANEL')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('tab', { name: /registers/i }));
    expect(screen.getByText('REG_PANEL')).toBeInTheDocument();
    expect(screen.queryByText('UART_PANEL')).toBeNull();
  });

  it('marks the active tab with aria-selected', () => {
    render(<DevDrawer devMode={true} tabs={tabs} />);
    const serialTab = screen.getByRole('tab', { name: /serial/i });
    expect(serialTab).toHaveAttribute('aria-selected', 'true');
  });
});
