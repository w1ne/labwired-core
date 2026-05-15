import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { PaletteDrawer, type PaletteComponent } from './PaletteDrawer';

const components: PaletteComponent[] = [
  { type: 'led', label: 'LED', category: 'gpio', bus: 'GPIO' },
  { type: 'adxl345', label: 'ADXL345', category: 'i2c', bus: 'I²C 0x53' },
  { type: 'bme280', label: 'BME280', category: 'i2c', bus: 'I²C 0x76' },
];

describe('PaletteDrawer', () => {
  it('starts closed (handle only)', () => {
    render(<PaletteDrawer components={components} open={false} onOpenChange={() => {}} onDragStart={() => {}} />);
    expect(screen.queryByRole('search')).toBeNull();
  });

  it('opens when open=true', () => {
    render(<PaletteDrawer components={components} open={true} onOpenChange={() => {}} onDragStart={() => {}} />);
    expect(screen.getByRole('search')).toBeInTheDocument();
  });

  it('filters by category tab', async () => {
    render(<PaletteDrawer components={components} open={true} onOpenChange={() => {}} onDragStart={() => {}} />);
    await userEvent.click(screen.getByRole('tab', { name: /^i.c$/i }));
    expect(screen.getByText('ADXL345')).toBeInTheDocument();
    expect(screen.queryByText('LED')).toBeNull();
  });

  it('filters by search query', async () => {
    render(<PaletteDrawer components={components} open={true} onOpenChange={() => {}} onDragStart={() => {}} />);
    await userEvent.type(screen.getByRole('searchbox'), 'bme');
    expect(screen.getByText('BME280')).toBeInTheDocument();
    expect(screen.queryByText('LED')).toBeNull();
  });

  it('toggles the drawer via the edge handle', async () => {
    const onOpenChange = vi.fn();
    render(<PaletteDrawer components={components} open={false} onOpenChange={onOpenChange} onDragStart={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: /open component palette/i }));
    expect(onOpenChange).toHaveBeenCalledWith(true);
  });
});
