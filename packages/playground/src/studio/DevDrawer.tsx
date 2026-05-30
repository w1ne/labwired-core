import { useState, type ReactNode } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';

export type DevTab = 'serial' | 'registers' | 'trace' | 'memory' | 'source' | 'yaml';

const TAB_ORDER: DevTab[] = ['serial', 'registers', 'trace', 'memory', 'source', 'yaml'];
const TAB_LABEL: Record<DevTab, string> = {
  serial: 'Serial',
  registers: 'Registers',
  trace: 'Trace',
  memory: 'Memory',
  source: 'Source',
  yaml: 'YAML',
};

export interface DevDrawerProps {
  devMode: boolean;
  tabs: Record<DevTab, ReactNode>;
  defaultHeight?: number;
  /** px to push the drawer's left edge in by, e.g. to clear the palette. */
  leftOffset?: number;
  /** Optional header row rendered immediately above the dev tab strip.
   *  Used by the multi-MCU PropertiesGate to host the chip-switcher
   *  tabs so they stick to the top of the drawer regardless of its
   *  resizable height. */
  header?: ReactNode;
}

export function DevDrawer({ devMode, tabs, defaultHeight = 240, leftOffset = 0, header }: DevDrawerProps) {
  const [active, setActive] = useState<DevTab>('serial');
  const [height, setHeight] = useState(defaultHeight);

  return (
    <AnimatePresence>
      {devMode && (
        <motion.div
          initial={{ y: height }}
          animate={{ y: 0 }}
          exit={{ y: height }}
          transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
          style={{ height, left: leftOffset, right: 0, transition: 'left 220ms cubic-bezier(0.16, 1, 0.3, 1)' }}
          className="absolute bottom-0 z-10 bg-bg-surface border-t border-border flex flex-col"
        >
          <div
            role="separator"
            aria-orientation="horizontal"
            aria-label="Resize dev drawer"
            onMouseDown={(event) => {
              event.preventDefault();
              const startY = event.clientY;
              const startHeight = height;
              const move = (e: MouseEvent) => {
                const next = Math.max(160, Math.min(window.innerHeight * 0.6, startHeight + (startY - e.clientY)));
                setHeight(next);
              };
              const up = () => {
                window.removeEventListener('mousemove', move);
                window.removeEventListener('mouseup', up);
              };
              window.addEventListener('mousemove', move);
              window.addEventListener('mouseup', up);
            }}
            className="h-1 cursor-ns-resize hover:bg-border"
          />
          <div role="tablist" className="flex items-center px-3 border-b border-border h-9 flex-shrink-0 overflow-x-auto">
            {header}
            {TAB_ORDER.map((tab) => (
              <button
                key={tab}
                role="tab"
                aria-selected={active === tab}
                onClick={() => setActive(tab)}
                className={clsx(
                  'h-9 px-3 text-[12px] font-medium border-b-2 transition-colors duration-micro',
                  active === tab
                    ? 'border-accent text-fg-primary'
                    : 'border-transparent text-fg-secondary hover:text-fg-primary'
                )}
              >
                {TAB_LABEL[tab]}
              </button>
            ))}
          </div>
          <div className="flex-1 overflow-auto">{tabs[active]}</div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
