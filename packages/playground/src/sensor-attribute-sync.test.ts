import { describe, expect, it, vi } from 'vitest';
import { syncSensorAttributeToSimulator } from './sensor-attribute-sync';

describe('syncSensorAttributeToSimulator', () => {
  it('pushes ultrasonic distance edits into the HC-SR04 simulator model', () => {
    const bridge = { setHcsr04Distance: vi.fn() };

    const synced = syncSensorAttributeToSimulator({
      partId: 'dist',
      partType: 'ultrasonic',
      key: 'distance',
      value: '42',
      bridge,
    });

    expect(synced).toBe(true);
    expect(bridge.setHcsr04Distance).toHaveBeenCalledWith('dist', 42);
  });

  it('ignores non-numeric ultrasonic distance edits', () => {
    const bridge = { setHcsr04Distance: vi.fn() };

    const synced = syncSensorAttributeToSimulator({
      partId: 'dist',
      partType: 'ultrasonic',
      key: 'distance',
      value: 'abc',
      bridge,
    });

    expect(synced).toBe(false);
    expect(bridge.setHcsr04Distance).not.toHaveBeenCalled();
  });
});
