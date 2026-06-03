import type { ReactNode } from 'react';
import type { Part } from '@labwired/ui';
import { Sn74hc165InputsControl } from './Sn74hc165InputsControl';

export interface ComponentRuntimeBridge {
  getSn74hc165Inputs?: () => number;
  setSn74hc165Inputs?: (value: number) => void;
}

export interface ComponentRuntimeControlContext {
  part: Part;
  bridge: ComponentRuntimeBridge | null;
  updateAttrs: (partId: string, attrs: Record<string, string>) => void;
}

type ComponentRuntimeControlRenderer = (context: ComponentRuntimeControlContext) => ReactNode;

function byteFromAttr(value: string | undefined, fallback = 165): number {
  const parsed = Number.parseInt(value ?? String(fallback), 10);
  if (!Number.isFinite(parsed)) return fallback;
  return Math.max(0, Math.min(255, Math.round(parsed)));
}

function renderSn74hc165Inputs({
  part,
  bridge,
  updateAttrs,
}: ComponentRuntimeControlContext): ReactNode {
  const bridgeValue = bridge?.getSn74hc165Inputs?.();
  const value = bridgeValue !== undefined && bridgeValue >= 0
    ? bridgeValue
    : byteFromAttr(part.attrs.inputs);

  const setInputs = (nextValue: number) => {
    const clamped = Math.max(0, Math.min(255, Math.round(nextValue)));
    updateAttrs(part.id, { inputs: String(clamped) });
    bridge?.setSn74hc165Inputs?.(clamped);
  };

  return (
    <Sn74hc165InputsControl
      value={value}
      onChannelChange={(channel, high) => {
        const nextValue = high ? value | (1 << channel) : value & ~(1 << channel);
        setInputs(nextValue);
      }}
      onByteChange={setInputs}
    />
  );
}

const RUNTIME_CONTROLS: Record<string, ComponentRuntimeControlRenderer> = {
  sn74hc165: renderSn74hc165Inputs,
};

export function renderComponentRuntimeControl(context: ComponentRuntimeControlContext): ReactNode {
  return RUNTIME_CONTROLS[context.part.type]?.(context);
}
