interface SensorAttributeSyncBridge {
  setHcsr04Distance?: (id: string, distanceCm: number) => void;
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
  if (partType !== 'ultrasonic' || key !== 'distance' || !bridge?.setHcsr04Distance) {
    return false;
  }

  const distanceCm = Number.parseFloat(value);
  if (!Number.isFinite(distanceCm)) {
    return false;
  }

  bridge.setHcsr04Distance(partId, distanceCm);
  return true;
}
