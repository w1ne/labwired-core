import { useMemo, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';

export type PaletteCategory = 'i2c' | 'spi' | 'uart' | 'analog' | 'gpio' | 'misc';

export interface PaletteComponent {
  type: string;
  label: string;
  category: PaletteCategory;
  bus?: string;
  thumb?: React.ReactNode;
}

export interface PaletteDrawerProps {
  components: PaletteComponent[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDragStart: (componentType: string) => void;
}

const CATEGORIES: { id: PaletteCategory | 'all'; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'i2c', label: 'I²C' },
  { id: 'spi', label: 'SPI' },
  { id: 'uart', label: 'UART' },
  { id: 'analog', label: 'Analog' },
  { id: 'gpio', label: 'GPIO' },
  { id: 'misc', label: 'Misc' },
];

export function PaletteDrawer({ components, open, onOpenChange, onDragStart }: PaletteDrawerProps) {
  const [activeCategory, setActiveCategory] = useState<PaletteCategory | 'all'>('all');
  const [query, setQuery] = useState('');

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return components.filter((component) => {
      if (activeCategory !== 'all' && component.category !== activeCategory) return false;
      if (!q) return true;
      return component.label.toLowerCase().includes(q) || component.type.includes(q);
    });
  }, [components, activeCategory, query]);

  return (
    <>
      <button
        type="button"
        aria-label="Open component palette"
        onClick={() => onOpenChange(!open)}
        className="absolute top-1/2 -translate-y-1/2 left-0 z-20 w-1.5 h-24 bg-border hover:bg-border-strong rounded-r-md transition-colors duration-micro"
      />
      <AnimatePresence>
        {open && (
          <motion.aside
            key="palette"
            initial={{ x: -280 }}
            animate={{ x: 0 }}
            exit={{ x: -280 }}
            transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
            className="absolute top-11 left-0 bottom-0 z-20 w-[280px] bg-bg-surface border-r border-border flex flex-col"
            aria-label="Component palette"
          >
            <div role="search" className="p-3 border-b border-border">
              <input
                type="search"
                role="searchbox"
                placeholder="Search components…"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                className="w-full h-8 px-2 rounded-button bg-bg-elevated border border-border text-fg-primary placeholder:text-fg-tertiary outline-none focus:border-accent"
              />
            </div>
            <div role="tablist" className="flex flex-wrap gap-1 px-3 py-2 border-b border-border">
              {CATEGORIES.map((cat) => (
                <button
                  key={cat.id}
                  role="tab"
                  aria-selected={activeCategory === cat.id}
                  onClick={() => setActiveCategory(cat.id)}
                  className={clsx(
                    'h-6 px-2 rounded-pill text-[11px] font-medium transition-colors duration-micro',
                    activeCategory === cat.id
                      ? 'bg-accent-soft text-accent border border-accent/40'
                      : 'text-fg-secondary hover:text-fg-primary border border-transparent'
                  )}
                >
                  {cat.label}
                </button>
              ))}
            </div>
            <div className="flex-1 overflow-y-auto p-2">
              {filtered.map((component) => (
                <div
                  key={component.type}
                  draggable
                  onDragStart={(event) => {
                    event.dataTransfer.setData('application/x-labwired-component', component.type);
                    event.dataTransfer.effectAllowed = 'copy';
                    onDragStart(component.type);
                  }}
                  className="flex items-center gap-3 px-2 py-2 rounded-button hover:bg-bg-elevated cursor-grab active:cursor-grabbing"
                >
                  <div className="w-8 h-8 rounded bg-bg-canvas border border-border flex items-center justify-center text-fg-secondary text-xs font-mono">
                    {component.thumb ?? component.type[0]?.toUpperCase()}
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="text-fg-primary text-[13px] truncate">{component.label}</div>
                    {component.bus && (
                      <div className="text-fg-tertiary text-[10px] font-mono truncate">{component.bus}</div>
                    )}
                  </div>
                </div>
              ))}
              {filtered.length === 0 && (
                <div className="text-fg-tertiary text-center mt-6 text-xs">No components match.</div>
              )}
            </div>
          </motion.aside>
        )}
      </AnimatePresence>
    </>
  );
}
