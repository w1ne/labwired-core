import { useState, useRef, useEffect } from 'react';
import { BOARD_CONFIGS, BOARD_CONFIG_MAP, type BoardConfig } from './bundled-configs';
import { type CatalogEntry, catalogSlug } from './catalog-client';

interface BoardPickerProps {
  catalog: CatalogEntry[];
  selectedBoardId: string;
  onSelect: (config: BoardConfig) => void;
}

/** Merge bundled configs with catalog entries for a unified board list. */
function buildBoardList(catalog: CatalogEntry[]) {
  // Start with bundled (simulatable) boards
  const items: BoardListItem[] = BOARD_CONFIGS.map((config) => {
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
      simulatable: true,
      hasDemoFw: !!config.demoFirmwarePath,
      config,
    };
  });

  // Add catalog-only entries (not simulatable)
  const bundledSlugs = new Set(BOARD_CONFIGS.flatMap((c) => [c.boardId, c.chipId]));
  for (const entry of catalog) {
    const slug = catalogSlug(entry.id);
    if (bundledSlugs.has(slug)) continue;
    // Only show boards and chips with register models
    if (!entry.id.startsWith('board/') && !entry.id.startsWith('chip/')) continue;
    if (entry.registers === 0 && !entry.image_url) continue; // skip low-quality entries
    items.push({
      id: slug,
      name: entry.name,
      description: entry.description,
      arch: entry.architecture || entry.family,
      imageUrl: entry.image_url,
      registers: entry.registers,
      verified: entry.verified,
      passRate: entry.pass_rate,
      simulatable: false,
      hasDemoFw: false,
      config: null,
    });
  }

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
  simulatable: boolean;
  hasDemoFw: boolean;
  config: BoardConfig | null;
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

  // Sort: simulatable first, then by name
  filtered.sort((a, b) => {
    if (a.simulatable !== b.simulatable) return a.simulatable ? -1 : 1;
    return a.name.localeCompare(b.name);
  });

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
            {filtered.map((item) => (
              <button
                key={item.id}
                className={`board-picker-item ${item.id === selectedBoardId ? 'selected' : ''} ${
                  !item.simulatable ? 'catalog-only' : ''
                }`}
                onClick={() => {
                  if (item.config) {
                    onSelect(item.config);
                    setOpen(false);
                    setFilter('');
                  }
                }}
                disabled={!item.simulatable}
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
                    {item.simulatable && (
                      <span className="badge badge-ready">Ready</span>
                    )}
                    {item.hasDemoFw && (
                      <span className="badge badge-demo">Demo</span>
                    )}
                    {!item.simulatable && (
                      <span className="badge badge-catalog">Catalog</span>
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
            {filtered.length === 0 && (
              <div className="board-picker-empty">No boards match "{filter}"</div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
