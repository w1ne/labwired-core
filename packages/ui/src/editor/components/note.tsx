import type { ComponentDef } from '../types';

const W = 220;
const PAD = 12;

/**
 * A free-form text annotation. Not a circuit element: no pins, no boardIoKind,
 * no wires — inert to diagramToConfig, validation, and the simulator. Text
 * lives in `attrs.text`. Rendered with a <foreignObject> so the body wraps and
 * the card grows in height (plain SVG <text> can't wrap). Inline editing is
 * handled in EditorCanvas (double-click), with a textarea fallback in the
 * PropertyPanel.
 */
export const noteComponent: ComponentDef = {
  type: 'note',
  label: 'Note',
  category: 'tool',
  width: W,
  height: 96, // nominal; real height comes from content via foreignObject
  pins: [],
  defaultAttrs: { text: 'Double-click to edit' },
  attrFields: [{ key: 'text', label: 'Text', type: 'textarea' }],
  render: (attrs, state) => {
    const text = attrs.text ?? '';
    const selected = !!state?.selected;
    return (
      <g>
        <foreignObject x={0} y={0} width={W} height={1} overflow="visible">
          <div
            // xmlns required so HTML inside SVG <foreignObject> paints in all browsers
            {...{ xmlns: 'http://www.w3.org/1999/xhtml' }}
            style={{
              width: `${W}px`,
              boxSizing: 'border-box',
              padding: `${PAD}px`,
              background: '#fff8e1',
              border: `1.5px solid ${selected ? '#F5B642' : '#e6d59a'}`,
              borderRadius: '8px',
              boxShadow: '0 1px 3px rgba(0,0,0,0.18)',
              font: "12px/1.45 -apple-system, 'Segoe UI', sans-serif",
              color: '#4a3f1e',
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
            }}
          >
            {text === '' ? ' ' : text}
          </div>
        </foreignObject>
      </g>
    );
  },
};
