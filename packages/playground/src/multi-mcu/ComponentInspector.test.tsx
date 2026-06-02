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

    const distanceInput = screen.getByLabelText('Distance (cm)');
    await user.clear(distanceInput);
    await user.type(distanceInput, '42');

    expect(onChange).toHaveBeenLastCalledWith('distance', '42');
  });
});
