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

  it('pushes 74HC165 input byte edits into the simulator model', () => {
    const bridge = { setSn74hc165Inputs: vi.fn() };

    const synced = syncSensorAttributeToSimulator({
      partId: 'di_shifter',
      partType: 'sn74hc165',
      key: 'inputs',
      value: '170',
      bridge,
    });

    expect(synced).toBe(true);
    expect(bridge.setSn74hc165Inputs).toHaveBeenCalledWith(170);
  });

  it('ignores invalid 74HC165 input byte edits', () => {
    const bridge = { setSn74hc165Inputs: vi.fn() };

    const synced = syncSensorAttributeToSimulator({
      partId: 'di_shifter',
      partType: 'sn74hc165',
      key: 'inputs',
      value: 'abc',
      bridge,
    });

    expect(synced).toBe(false);
    expect(bridge.setSn74hc165Inputs).not.toHaveBeenCalled();
  });
});
