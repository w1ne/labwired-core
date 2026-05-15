import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { InspectorCard, type InspectorSelection } from './InspectorCard';

const partSelection: InspectorSelection = {
  kind: 'part',
  partId: 'adxl345',
  partType: 'adxl345',
  label: 'ADXL345',
  pins: [
    { id: 'VCC', label: '3v3' },
    { id: 'GND', label: 'GND' },
    { id: 'SDA', label: 'PB7' },
    { id: 'SCL', label: 'PB6' },
  ],
  attrs: {},
};

const wireSelection: InspectorSelection = {
  kind: 'wire',
  wireId: 'wire-1',
  from: 'mcu:PA5',
  to: 'led_pa5:A',
  color: '#3DD68C',
};

describe('InspectorCard', () => {
  it('renders nothing when no selection', () => {
    render(<InspectorCard selection={null} devMode={false} onDelete={() => {}} onDuplicate={() => {}} />);
    expect(screen.queryByRole('complementary')).toBeNull();
  });

  it('shows the selected part label and id', () => {
    render(<InspectorCard selection={partSelection} devMode={false} onDelete={() => {}} onDuplicate={() => {}} />);
    expect(screen.getByText('ADXL345')).toBeInTheDocument();
    expect(screen.getByText('adxl345')).toBeInTheDocument();
  });

  it('renders the pin table', () => {
    render(<InspectorCard selection={partSelection} devMode={false} onDelete={() => {}} onDuplicate={() => {}} />);
    expect(screen.getByText('SDA')).toBeInTheDocument();
    expect(screen.getByText('PB7')).toBeInTheDocument();
  });

  it('invokes onDelete when delete is clicked', async () => {
    const onDelete = vi.fn();
    render(<InspectorCard selection={partSelection} devMode={false} onDelete={onDelete} onDuplicate={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: /^delete$/i }));
    expect(onDelete).toHaveBeenCalledWith('adxl345');
  });

  it('hides the advanced toggle when dev mode is off', () => {
    render(<InspectorCard selection={partSelection} devMode={false} advancedView={<div>regs</div>} onDelete={() => {}} onDuplicate={() => {}} />);
    expect(screen.queryByRole('button', { name: /advanced/i })).toBeNull();
  });

  it('renders the wire inspector for wire selection', () => {
    render(<InspectorCard selection={wireSelection} devMode={false} onDelete={() => {}} onDuplicate={() => {}} />);
    expect(screen.getByText(/mcu:PA5 → led_pa5:A/i)).toBeInTheDocument();
  });

  it('renders the labWidget slot when provided', () => {
    render(
      <InspectorCard
        selection={partSelection}
        devMode={false}
        labWidget={<div data-testid="lab-widget">ADXL sliders</div>}
        onDelete={() => {}}
        onDuplicate={() => {}}
      />
    );
    expect(screen.getByTestId('lab-widget')).toBeInTheDocument();
  });
});
