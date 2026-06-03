interface SensorAttributeSyncBridge {
  setHcsr04Distance?: (id: string, distanceCm: number) => void;
  setSn74hc165Inputs?: (value: number) => void;
}

interface SensorAttributeSyncArgs {
  partId: string;
  partType: string;
  key: string;
  value: string;
  bridge: SensorAttributeSyncBridge | null | undefined;
}

export function syncSensorAttributeToSimulator({
  partId,
  partType,
  key,
  value,
  bridge,
}: SensorAttributeSyncArgs): boolean {
  if (partType === 'ultrasonic' && key === 'distance' && bridge?.setHcsr04Distance) {
    const distanceCm = Number.parseFloat(value);
    if (!Number.isFinite(distanceCm)) {
      return false;
    }

    bridge.setHcsr04Distance(partId, distanceCm);
    return true;
  }

  if (partType === 'sn74hc165' && key === 'inputs' && bridge?.setSn74hc165Inputs) {
    const inputs = Number.parseInt(value, 10);
    if (!Number.isFinite(inputs)) {
      return false;
    }

    bridge.setSn74hc165Inputs(Math.max(0, Math.min(255, inputs)));
    return true;
  }

  return false;
}
