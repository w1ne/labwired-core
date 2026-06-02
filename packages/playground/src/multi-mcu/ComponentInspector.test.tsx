import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ComponentInspector } from './ComponentInspector';

describe('ComponentInspector', () => {
  it('renders live controls alongside editable attributes', () => {
    render(
      <ComponentInspector
        partType="ultrasonic"
        partId="dist"
        attrs={{ distance: '30' }}
        fields={[{ key: 'distance', label: 'Distance (cm)', type: 'text' }]}
        onChange={() => {}}
        liveControl={<label>Hand distance</label>}
      />,
    );

    expect(screen.getByText('Distance (cm)')).toBeInTheDocument();
    expect(screen.getByText('Hand distance')).toBeInTheDocument();
  });
});
