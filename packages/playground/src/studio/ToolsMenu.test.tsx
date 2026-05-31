import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/react';
import { ToolsMenu } from './ToolsMenu';

afterEach(() => cleanup());

describe('ToolsMenu', () => {
  it('renders the Tools trigger and opens the dropdown listing tools', () => {
    render(
      <ToolsMenu
        tools={[
          {
            id: 'air-tracer',
            label: 'Air Tracer · BLE',
            description: 'Catch virtual-air frames (CRC)',
            active: false,
            onToggle: () => {},
          },
        ]}
      />,
    );

    const trigger = screen.getByTitle('Tools');
    expect(trigger).toBeTruthy();
    // Dropdown is closed initially.
    expect(screen.queryByText('Air Tracer · BLE')).toBeNull();

    fireEvent.click(trigger);
    expect(screen.getByText('Air Tracer · BLE')).toBeTruthy();
    expect(screen.getByText('Catch virtual-air frames (CRC)')).toBeTruthy();
  });

  it('calls onToggle when a tool row is clicked', () => {
    const onToggle = vi.fn();
    render(
      <ToolsMenu
        tools={[
          {
            id: 'air-tracer',
            label: 'Air Tracer · BLE',
            active: false,
            onToggle,
          },
        ]}
      />,
    );

    fireEvent.click(screen.getByTitle('Tools'));
    fireEvent.click(screen.getByText('Air Tracer · BLE'));
    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  it('closes the dropdown on Escape', () => {
    render(
      <ToolsMenu
        tools={[
          { id: 'air-tracer', label: 'Air Tracer · BLE', active: false, onToggle: () => {} },
        ]}
      />,
    );

    fireEvent.click(screen.getByTitle('Tools'));
    expect(screen.getByText('Air Tracer · BLE')).toBeTruthy();

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByText('Air Tracer · BLE')).toBeNull();
  });
});
