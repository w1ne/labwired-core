import { useEffect, useRef, useState } from 'react';
import './ToolsMenu.css';

export interface ToolItem {
  /** Stable identifier (used as React key). */
  id: string;
  /** Primary label shown in the menu row. */
  label: string;
  /** Optional one-line description shown under the label. */
  description?: string;
  /** Whether the tool is currently active/open. */
  active: boolean;
  /** Toggle handler for the tool. */
  onToggle: () => void;
}

interface ToolsMenuProps {
  tools: ToolItem[];
  openSignal?: number;
}

function WrenchGlyph() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18v3h3l6.3-6.3a4 4 0 0 0 5.4-5.4l-2.7 2.7-2-2 2.7-2.7z" />
    </svg>
  );
}

function CaretGlyph() {
  return (
    <svg
      className="tools-menu-caret"
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

/**
 * Toolbar dropdown that houses optional studio tools (e.g. the Air Tracer).
 * Add new tools by appending to the `tools` array passed in — each renders
 * as a toggleable menu row.
 */
export function ToolsMenu({ tools, openSignal }: ToolsMenuProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const lastOpenSignalRef = useRef(openSignal);

  const anyActive = tools.some((t) => t.active);

  useEffect(() => {
    if (openSignal === undefined) return;
    if (lastOpenSignalRef.current === openSignal) return;
    lastOpenSignalRef.current = openSignal;
    setOpen(true);
  }, [openSignal]);

  useEffect(() => {
    if (!open) return;
    function handleClickOutside(event: MouseEvent) {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    }
    function handleEscape(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        setOpen(false);
      }
    }
    document.addEventListener('mousedown', handleClickOutside);
    document.addEventListener('keydown', handleEscape);
    return () => {
      document.removeEventListener('mousedown', handleClickOutside);
      document.removeEventListener('keydown', handleEscape);
    };
  }, [open]);

  return (
    <div className="tools-menu" ref={rootRef}>
      <button
        type="button"
        className={`toolbar-btn toolbar-btn-ghost tools-menu-trigger ${open || anyActive ? 'active' : ''}`}
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        title="Tools"
      >
        <WrenchGlyph />
        <span className="tools-menu-label">Tools</span>
        <CaretGlyph />
      </button>

      {open && (
        <div className="tools-menu-panel" role="menu">
          {tools.map((tool) => (
            <button
              key={tool.id}
              type="button"
              role="menuitemcheckbox"
              aria-checked={tool.active}
              className={`tools-menu-item ${tool.active ? 'active' : ''}`}
              onClick={() => tool.onToggle()}
            >
              <span className="tools-menu-item-check" aria-hidden="true">
                {tool.active ? '✓' : ''}
              </span>
              <span className="tools-menu-item-text">
                <span className="tools-menu-item-label">{tool.label}</span>
                {tool.description && (
                  <span className="tools-menu-item-desc">{tool.description}</span>
                )}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
