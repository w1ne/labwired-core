import { useState, useRef, useEffect } from 'react';
import { BOARD_CONFIGS, BOARD_CONFIG_MAP, type BoardConfig } from './bundled-configs';
import { type CatalogEntry, catalogSlug } from './catalog-client';
// Catalog entries are used only to enrich bundled boards with images/metadata

interface BoardPickerProps {
  catalog: CatalogEntry[];
  selectedBoardId: string;
  onSelect: (config: BoardConfig) => void;
}

/** Merge bundled configs with catalog entries for a unified board list. */
function buildBoardList(catalog: CatalogEntry[]) {
  // Start with bundled (simulatable) boards
  const items: BoardListItem[] = BOARD_CONFIGS.filter((c) => !c.hidden).map((config) => {
    // Find matching catalog entry for image/metadata enrichment
    const catalogEntry = catalog.find(
      (c) => catalogSlug(c.id) === config.boardId || catalogSlug(c.id) === config.chipId,
    );
    return {
      id: config.boardId,
      name: config.name,
      description: config.description,
      arch: config.arch,
      imageUrl: catalogEntry?.image_url ?? '',
      registers: catalogEntry?.registers ?? 0,
      verified: catalogEntry?.verified ?? false,
      passRate: catalogEntry?.pass_rate ?? 0,
      hasDemoFw: !!config.demoFirmwarePath,
      config,
    };
  });

  return items;
}

interface BoardListItem {
  id: string;
  name: string;
  description: string;
  arch: string;
  imageUrl: string;
  registers: number;
  verified: boolean;
  passRate: number;
  hasDemoFw: boolean;
  config: BoardConfig;
}

export function BoardPicker({ catalog, selectedBoardId, onSelect }: BoardPickerProps) {
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState('');
  const ref = useRef<HTMLDivElement>(null);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  const items = buildBoardList(catalog);
  const filtered = filter
    ? items.filter((b) => b.name.toLowerCase().includes(filter.toLowerCase()))
    : items;

  filtered.sort((a, b) => a.name.localeCompare(b.name));

  // Split into Labs (pre-wired, ready to Run) and Bare boards (MCU only —
  // you wire it). Renders as two sections with sticky headers so visitors
  // can see at a glance which entries are demo-ready.
  const labs = filtered.filter((b) => b.config.kind === 'lab');
  const bares = filtered.filter((b) => b.config.kind !== 'lab');

  const selected = BOARD_CONFIG_MAP.get(selectedBoardId);

  return (
    <div className="board-picker" ref={ref}>
      <button className="board-picker-trigger" onClick={() => setOpen(!open)}>
        <span className="board-picker-name">{selected?.name ?? 'Select Board'}</span>
        <span className="board-picker-arch">{selected?.arch ?? ''}</span>
        <span className="board-picker-caret">{open ? '\u25B2' : '\u25BC'}</span>
      </button>

      {open && (
        <div className="board-picker-dropdown">
          <input
            className="board-picker-search"
            placeholder="Filter boards..."
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            autoFocus
          />
          <div className="board-picker-list">
            {[
              { label: 'Labs · pre-wired projects', items: labs },
              { label: 'Bare boards · MCU only', items: bares },
            ].map((section) =>
              section.items.length === 0 ? null : (
                <div key={section.label} className="board-picker-section">
                  <div className="board-picker-section-header">{section.label}</div>
                  {section.items.map((item) => (
                    <button
                      key={item.id}
                      className={`board-picker-item ${item.id === selectedBoardId ? 'selected' : ''}`}
                      onClick={() => {
                        onSelect(item.config);
                        setOpen(false);
                        setFilter('');
                      }}
                    >
                      <div className="board-picker-img">
                        {item.imageUrl ? (
                          <img
                            src={item.imageUrl}
                            alt={item.name}
                            onError={(e) => { (e.target as HTMLImageElement).style.display = 'none'; }}
                          />
                        ) : (
                          <div className="board-picker-chip-icon" />
                        )}
                      </div>
                      <div className="board-picker-info">
                        <div className="board-picker-item-name">
                          {item.name}
                          {item.config.kind === 'lab' && (
                            <span className="badge badge-demo">Lab</span>
                          )}
                          {item.config.kind !== 'lab' && item.hasDemoFw && (
                            <span className="badge badge-demo">Demo</span>
                          )}
                        </div>
                        <div className="board-picker-item-meta">
                          {item.arch && <span>{item.arch}</span>}
                          {item.registers > 0 && <span>{item.registers} regs</span>}
                          {item.verified && <span className="verified-badge">Verified</span>}
                        </div>
                      </div>
                    </button>
                  ))}
                </div>
              ),
            )}
            {filtered.length === 0 && (
              <div className="board-picker-empty">No boards match "{filter}"</div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
