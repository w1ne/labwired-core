import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { ComponentInspector } from './ComponentInspector';

describe('ComponentInspector', () => {
  it('does not render duplicate runtime controls outside the component attributes', () => {
    const Inspector = ComponentInspector as unknown as (props: Parameters<typeof ComponentInspector>[0] & { liveControl?: ReactNode }) => ReactElement;

    render(
      <Inspector
        partType="ultrasonic"
        partId="dist"
        attrs={{ distance: '30' }}
        fields={[{ key: 'distance', label: 'Distance (cm)', type: 'text' }]}
        onChange={() => {}}
        liveControl={<label>Hand distance</label>}
      />,
    );

    expect(screen.getByText('Distance (cm)')).toBeInTheDocument();
    expect(screen.queryByText('Hand distance')).not.toBeInTheDocument();
  });

  it('renders explicit runtime controls inside the properties body', async () => {
    const user = userEvent.setup();
    const onToggle = vi.fn();

    render(
      <ComponentInspector
        partType="sn74hc165"
        partId="di_shifter"
        attrs={{}}
        fields={[]}
        onChange={() => {}}
        runtimeControl={
          <button type="button" onClick={() => onToggle(2, true)}>
            D2 HI
          </button>
        }
      />,
    );

    await user.click(screen.getByRole('button', { name: 'D2 HI' }));

    expect(screen.getByText('No editable properties.')).toBeInTheDocument();
    expect(onToggle).toHaveBeenCalledWith(2, true);
  });

  it('edits the component distance attribute through the single distance field', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    function Harness() {
      const [distance, setDistance] = useState('30');
      return (
        <ComponentInspector
          partType="ultrasonic"
          partId="dist"
          attrs={{ distance }}
          fields={[{ key: 'distance', label: 'Distance (cm)', type: 'text' }]}
          onChange={(key, value) => {
            onChange(key, value);
            setDistance(value);
          }}
        />
      );
    }

    render(<Harness />);

    const distanceInput = screen.getByRole('textbox', { name: 'Distance (cm)' });
    await user.clear(distanceInput);
    await user.type(distanceInput, '42');

    expect(onChange).toHaveBeenLastCalledWith('distance', '42');
  });

  it('renders range attributes as a synced slider and input', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    function Harness() {
      const [distance, setDistance] = useState('30');
      return (
        <ComponentInspector
          partType="ultrasonic"
          partId="dist"
          attrs={{ distance }}
          fields={[{ key: 'distance', label: 'Distance (cm)', type: 'range', min: 1, max: 200, step: 1 }]}
          onChange={(key, value) => {
            onChange(key, value);
            setDistance(value);
          }}
        />
      );
    }

    render(<Harness />);

    const distanceInput = screen.getByRole('textbox', { name: 'Distance (cm)' });
    const distanceSlider = screen.getByRole('slider', { name: 'Distance (cm)' });

    expect(distanceInput).toHaveValue('30');
    expect(distanceSlider).toHaveValue('30');

    await user.clear(distanceInput);
    await user.type(distanceInput, '42');
    expect(distanceSlider).toHaveValue('42');
    expect(onChange).toHaveBeenLastCalledWith('distance', '42');

    await user.clear(distanceInput);
    await user.type(distanceInput, '200');
    expect(distanceSlider).toHaveValue('200');
    expect(onChange).toHaveBeenLastCalledWith('distance', '200');
  });

  it('uses the range default value when the attribute is missing', () => {
    render(
      <ComponentInspector
        partType="ultrasonic"
        partId="dist"
        attrs={{}}
        fields={[{ key: 'distance', label: 'Distance (cm)', type: 'range', min: 1, max: 200, step: 1, defaultValue: '100' }]}
        onChange={() => {}}
      />,
    );

    expect(screen.getByRole('textbox', { name: 'Distance (cm)' })).toHaveValue('100');
    expect(screen.getByRole('slider', { name: 'Distance (cm)' })).toHaveValue('100');
  });
});
