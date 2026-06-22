// Generic, data-driven component support. A part can be defined purely by DATA
// (pins + metadata, no bespoke SVG): `defineComponent({...})` fills a generic
// box+labels renderer when none is given, so ANY part renders on the board.
// `genericComponentDef` synthesizes a def for an unregistered type so nothing
// ever fails to draw. `renderComponentBody` is what the canvas calls.
import type { ReactNode } from 'react';
import type { ComponentDef, ComponentState, PinDef } from '../types';

const PIN_LABEL_FONT = 9;

/** A generic SVG body: rounded rect + centred label + per-pin labels by side. */
export function makeGenericRender(
  def: Pick<ComponentDef, 'width' | 'height' | 'label' | 'pins' | 'category'>,
): (attrs: Record<string, string>, state?: ComponentState) => ReactNode {
  return (_attrs, state) => {
    const selected = state?.selected ?? false;
    const w = def.width;
    const h = def.height;
    return (
      <g>
        <rect
          x={0}
          y={0}
          width={w}
          height={h}
          rx={6}
          fill="var(--lw-bg-surface, #15151d)"
          stroke={selected ? 'var(--lw-accent, #34d399)' : 'var(--lw-border, #3a3a46)'}
          strokeWidth={selected ? 2 : 1.5}
        />
        <text
          x={w / 2}
          y={h / 2}
          textAnchor="middle"
          dominantBaseline="central"
          fontFamily="ui-sans-serif, system-ui, sans-serif"
          fontSize={Math.min(13, Math.max(9, w / Math.max(6, def.label.length)))}
          fontWeight={700}
          fill="#e4e4e7"
        >
          {def.label}
        </text>
        {def.pins.map((pin: PinDef) => {
          const txt = pin.label ?? pin.id;
          // Place the label just inside the body, offset from the pin by side.
          const anchor = pin.side === 'left' ? 'start' : pin.side === 'right' ? 'end' : 'middle';
          const dx = pin.side === 'left' ? 6 : pin.side === 'right' ? -6 : 0;
          const dy = pin.side === 'top' ? 10 : pin.side === 'bottom' ? -10 : 0;
          return (
            <text
              key={pin.id}
              x={pin.x + dx}
              y={pin.y + dy}
              textAnchor={anchor}
              dominantBaseline="central"
              fontFamily="ui-monospace, monospace"
              fontSize={PIN_LABEL_FONT}
              fill="#8a8a99"
            >
              {txt}
            </text>
          );
        })}
      </g>
    );
  };
}

/** Render a component's body, falling back to the generic renderer. */
export function renderComponentBody(
  def: ComponentDef,
  attrs: Record<string, string>,
  state?: ComponentState,
): ReactNode {
  const render = def.render ?? makeGenericRender(def);
  return render(attrs, state);
}

/** Auto-size a box from its pin count so labels fit (used when no width/height given). */
function autoSize(pins: PinDef[]): { width: number; height: number } {
  const perSide: Record<string, number> = { left: 0, right: 0, top: 0, bottom: 0 };
  for (const p of pins) perSide[p.side] = (perSide[p.side] ?? 0) + 1;
  const rows = Math.max(perSide.left, perSide.right, 1);
  const cols = Math.max(perSide.top, perSide.bottom, 1);
  return { width: Math.max(120, cols * 36), height: Math.max(56, rows * 26) };
}

/** Build a full ComponentDef from data; generic render + defaults when omitted. */
export function defineComponent(
  data: Partial<ComponentDef> & { type: string; label: string; pins?: PinDef[] },
): ComponentDef {
  const pins = data.pins ?? [];
  const size = autoSize(pins);
  const def: ComponentDef = {
    type: data.type,
    label: data.label,
    category: data.category ?? 'ic',
    width: data.width ?? size.width,
    height: data.height ?? size.height,
    pins,
    defaultAttrs: data.defaultAttrs ?? {},
    deviceClass: data.deviceClass,
    boardIoKind: data.boardIoKind,
    attrFields: data.attrFields,
    render: data.render,
  };
  if (!def.render) def.render = makeGenericRender(def);
  return def;
}

/** A synthesized def for an UNREGISTERED type so it still draws (box, no pins). */
export function genericComponentDef(type: string): ComponentDef {
  return defineComponent({ type, label: type, category: 'ic', pins: [], width: 120, height: 56 });
}
