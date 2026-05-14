import { useEffect } from 'react';
import { Command } from 'cmdk';
import { motion, AnimatePresence } from 'framer-motion';

export type CommandBucket = 'Components' | 'Boards' | 'Examples' | 'Actions';

export interface CommandItem {
  id: string;
  bucket: CommandBucket;
  label: string;
  hint?: string;
  action: () => void;
}

export interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
  items: CommandItem[];
}

const BUCKETS: CommandBucket[] = ['Components', 'Boards', 'Examples', 'Actions'];

export function CommandPalette({ open, onClose, items }: CommandPaletteProps) {
  useEffect(() => {
    if (!open) return;
    const handler = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.stopPropagation();
        onClose();
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [open, onClose]);

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.16 }}
          className="fixed inset-0 z-50 flex items-start justify-center pt-[18vh] bg-bg-base/60 backdrop-blur"
          onClick={onClose}
        >
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-label="Command palette"
            initial={{ opacity: 0, y: -8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8 }}
            transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
            className="lw-glass w-[min(560px,calc(100vw-32px))] overflow-hidden"
            onClick={(event) => event.stopPropagation()}
          >
            <Command loop>
              <div className="h-14 px-5 flex items-center gap-3 border-b border-border">
                <span className="text-fg-tertiary text-base" aria-hidden>⌘</span>
                <Command.Input
                  autoFocus
                  placeholder="Search components, boards, examples…"
                  className="flex-1 bg-transparent outline-none text-[15px] placeholder:text-fg-tertiary"
                />
              </div>
              <Command.List className="max-h-[60vh] overflow-y-auto py-2">
                <Command.Empty className="px-5 py-6 text-fg-tertiary text-center text-sm">
                  No matches.
                </Command.Empty>
                {BUCKETS.map((bucket) => {
                  const inBucket = items.filter((item) => item.bucket === bucket);
                  if (inBucket.length === 0) return null;
                  return (
                    <Command.Group
                      key={bucket}
                      heading={bucket}
                      className="text-fg-tertiary text-[10px] uppercase tracking-wider px-3 py-1"
                    >
                      {inBucket.map((item) => (
                        <Command.Item
                          key={item.id}
                          value={`${item.bucket} ${item.label}`}
                          onSelect={() => {
                            item.action();
                            onClose();
                          }}
                          className="flex items-center justify-between px-3 py-2 text-fg-primary text-[13px] aria-selected:bg-accent-soft aria-selected:text-accent cursor-pointer rounded"
                        >
                          <span>{item.label}</span>
                          {item.hint && <span className="text-fg-tertiary text-[11px]">{item.hint}</span>}
                        </Command.Item>
                      ))}
                    </Command.Group>
                  );
                })}
              </Command.List>
            </Command>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
