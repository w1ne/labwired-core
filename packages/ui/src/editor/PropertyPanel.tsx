import type { ReactNode } from 'react';
import type { Part } from './types';
import { COMPONENT_REGISTRY } from './components/index';

interface PropertyPanelProps {
  parts: Part[];
  onUpdateAttrs: (id: string, attrs: Record<string, string>) => void;
  onDelete: () => void;
  onRotate: (id: string) => void;
  onResize?: (id: string, scale: number) => void;
  /** Live, simulation-driven widget for the selected part (sensor sliders,
   *  display framebuffers, etc.). Rendered below the static attributes. */
  labWidget?: ReactNode;
}

export function PropertyPanel({ parts, onUpdateAttrs, onDelete, onRotate, onResize, labWidget }: PropertyPanelProps) {
  if (parts.length === 0) {
    return (
      <div className="editor-property-panel">
        <div className="panel-empty">Select a component to edit its properties</div>
      </div>
    );
  }

  if (parts.length > 1) {
    return (
      <div className="editor-property-panel">
        <h3 className="panel-title">{parts.length} selected</h3>
        <div className="panel-actions">
          <button className="panel-btn panel-btn-danger" onClick={onDelete} title="Delete selected">
            Delete All
          </button>
        </div>
      </div>
    );
  }

  const part = parts[0];
  const def = COMPONENT_REGISTRY.get(part.type);
  if (!def) return null;

  return (
    <div className="editor-property-panel">
      <h3 className="panel-title">{def.label}</h3>
      <div className="panel-id">ID: {part.id}</div>

      <div className="panel-section">
        <div className="panel-row">
          <label>X</label>
          <input
            type="number"
            value={part.x}
            readOnly
            className="panel-input panel-input-sm"
          />
        </div>
        <div className="panel-row">
          <label>Y</label>
          <input
            type="number"
            value={part.y}
            readOnly
            className="panel-input panel-input-sm"
          />
        </div>
        <div className="panel-row">
          <label>Rotation</label>
          <span className="panel-value">{part.rotate}°</span>
        </div>
        <div className="panel-row">
          <label>Scale</label>
          <input
            type="range"
            min="0.3"
            max="4"
            step="0.1"
            value={part.scale ?? 1}
            className="panel-slider"
            onChange={(e) => onResize?.(part.id, parseFloat(e.target.value))}
          />
          <span className="panel-value">{Math.round((part.scale ?? 1) * 100)}%</span>
        </div>
      </div>

      {def.attrFields && def.attrFields.length > 0 && (
        <div className="panel-section">
          <div className="panel-section-title">Attributes</div>
          {def.attrFields.map((field) => (
            <div key={field.key} className="panel-row">
              <label>{field.label}</label>
              {field.type === 'select' && field.options ? (
                <select
                  className="panel-input"
                  value={part.attrs[field.key] || ''}
                  onChange={(e) =>
                    onUpdateAttrs(part.id, { [field.key]: e.target.value })
                  }
                >
                  {field.options.map((opt) => (
                    <option key={opt} value={opt}>{opt}</option>
                  ))}
                </select>
              ) : field.type === 'range' ? (
                <>
                  <input
                    type="range"
                    min={field.min}
                    max={field.max}
                    step={field.step}
                    className="panel-slider"
                    value={part.attrs[field.key] ?? field.defaultValue ?? ''}
                    aria-label={field.label}
                    onChange={(e) =>
                      onUpdateAttrs(part.id, { [field.key]: e.target.value })
                    }
                  />
                  <input
                    type="text"
                    inputMode="decimal"
                    className="panel-input panel-input-sm"
                    value={part.attrs[field.key] ?? field.defaultValue ?? ''}
                    onChange={(e) =>
                      onUpdateAttrs(part.id, { [field.key]: e.target.value })
                    }
                  />
                </>
              ) : (
                <input
                  type="text"
                  className="panel-input"
                  value={part.attrs[field.key] || ''}
                  onChange={(e) =>
                    onUpdateAttrs(part.id, { [field.key]: e.target.value })
                  }
                />
              )}
            </div>
          ))}
        </div>
      )}

      {labWidget && (
        <div className="panel-section">
          <div className="panel-section-title">Live</div>
          {labWidget}
        </div>
      )}

      <div className="panel-actions">
        <button className="panel-btn" onClick={() => onRotate(part.id)} title="Rotate 90°">
          ↻ Rotate
        </button>
        <button className="panel-btn panel-btn-danger" onClick={onDelete} title="Delete component">
          Delete
        </button>
      </div>
    </div>
  );
}
