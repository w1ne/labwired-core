import type { Diagram, SimulatorBridge } from '@labwired/ui';
import { IoLinkAnalyzer } from './IoLinkAnalyzer';
import { getIolinkDecoderBinding, getLogicAnalyzerChannelBindings } from './logicAnalyzerConnections';

export interface LogicAnalyzerPanelProps {
  diagram: Diagram;
  analyzerId: string;
  bridge: SimulatorBridge | null;
  running: boolean;
}

export function LogicAnalyzerPanel({ diagram, analyzerId, bridge, running }: LogicAnalyzerPanelProps) {
  const bindings = getLogicAnalyzerChannelBindings(diagram, analyzerId);
  const iolink = getIolinkDecoderBinding(diagram, analyzerId);

  return (
    <div className="flex h-full min-h-0 flex-col text-[12px] text-fg-primary">
      <div className="border-b border-border px-3 py-2">
        <div className="grid grid-cols-4 gap-1">
          {bindings.map((binding) => {
            const label = binding.endpoints.length
              ? binding.endpoints.map((endpoint) => `${endpoint.part}.${endpoint.pin}`).join(', ')
              : 'not connected';
            return (
              <div key={binding.channel} className="min-w-0 rounded bg-bg-canvas px-2 py-1">
                <div className="font-mono text-[10px] font-semibold text-fg-secondary">{binding.channel}</div>
                <div className="truncate font-mono text-[10px] text-fg-primary" title={label}>
                  {label}
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {iolink.connected ? (
        <>
          <div className="flex items-center justify-between border-b border-border px-3 py-1.5 font-mono text-[11px] text-fg-secondary">
            <span>
              IO-Link decoder armed
            </span>
            <span>
              {iolink.channels.map((channel) => `${channel.channel}:${channel.pin}`).join('  ')}
            </span>
          </div>
          <div className="min-h-0 flex-1">
            <IoLinkAnalyzer bridge={bridge} running={running} />
          </div>
        </>
      ) : (
        <div className="flex flex-1 items-center justify-center px-6 text-center text-[12px] text-fg-tertiary">
          Connect a channel to the IO-Link TX or RX net to decode traffic.
        </div>
      )}
    </div>
  );
}
