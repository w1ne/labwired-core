import { getComponentsByCategory } from './components/index';

const CATEGORY_LABELS: Record<string, string> = {
  output: 'Output',
  input: 'Input',
  passive: 'Passive',
  sensor: 'Sensors',
  display: 'Displays',
  ic: 'ICs',
  tool: 'Tools',
};

const CATEGORY_ORDER = ['output', 'input', 'sensor', 'display', 'passive', 'ic', 'tool'];

interface ComponentPaletteProps {
  onAddPart?: (type: string) => void;
}

export function ComponentPalette({ onAddPart }: ComponentPaletteProps) {
  const groups = getComponentsByCategory();

  const handleDragStart = (e: React.DragEvent, type: string) => {
    e.dataTransfer.setData('application/x-component-type', type);
    e.dataTransfer.effectAllowed = 'copy';
  };

  return (
    <div className="editor-palette">
      <h3 className="palette-title">Components</h3>
      {CATEGORY_ORDER.filter((cat) => groups[cat]).map((cat) => [cat, groups[cat]] as const).map(([cat, defs]) => (
        <div key={cat} className="palette-group">
          <div className="palette-category">{CATEGORY_LABELS[cat] || cat}</div>
          {defs.map((def) => (
            <div
              key={def.type}
              className="palette-item"
              draggable
              onDragStart={(e) => handleDragStart(e, def.type)}
              onClick={() => onAddPart?.(def.type)}
              title={`Drag or click to add ${def.label}`}
            >
              <svg
                width={32}
                height={32}
                viewBox={`0 0 ${def.width} ${def.height}`}
                style={{ flexShrink: 0 }}
              >
                {def.render(def.defaultAttrs, { id: `palette-${def.type}` })}
              </svg>
              <span className="palette-label">{def.label}</span>
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}
