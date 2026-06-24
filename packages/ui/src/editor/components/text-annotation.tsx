import type { ComponentDef } from '../types';

const W = 96;
const H = 24;

/**
 * Non-electrical free-text annotation. Lets the user label or comment on a
 * circuit. It has no pins and no `boardIoKind`, so every wire/ERC path treats
 * it as inert decoration. The text content is edited from the inspector via the
 * `text` attrField, with an optional font-size range.
 */
export const textAnnotationComponent: ComponentDef = {
  type: 'text-annotation',
  label: 'Text / Comment',
  category: 'tool',
  width: W,
  height: H,
  pins: [],
  defaultAttrs: { text: 'Note', fontSize: '14' },
  attrFields: [
    { key: 'text', label: 'Text', type: 'text', defaultValue: 'Note' },
    { key: 'fontSize', label: 'Font size', type: 'range', min: 8, max: 48, step: 1, defaultValue: '14' },
  ],
  render: (attrs, state) => {
    const selected = !!state?.selected;
    const text = attrs.text ?? 'Note';
    const fontSizeRaw = Number(attrs.fontSize);
    const fontSize = Number.isFinite(fontSizeRaw) && fontSizeRaw > 0 ? fontSizeRaw : 14;
    return (
      <g>
        {/* Generous transparent hit area so the text is easy to grab/click. */}
        <rect x={0} y={0} width={W} height={H} fill="transparent" pointerEvents="all" />
        {selected && (
          <rect
            x={0}
            y={0}
            width={W}
            height={H}
            rx={3}
            fill="none"
            stroke="#e83e8c"
            strokeWidth={1.5}
            strokeDasharray="3 2"
          />
        )}
        <text
          x={4}
          y={H / 2}
          dominantBaseline="central"
          fill="#e6edf3"
          fontFamily="'JetBrains Mono', monospace"
          fontSize={fontSize}
        >
          {text}
        </text>
      </g>
    );
  },
};
