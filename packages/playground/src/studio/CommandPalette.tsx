import { useEffect } from 'react';
import { Command } from 'cmdk';
import { motion, AnimatePresence } from 'framer-motion';

export type CommandMode = 'search' | 'assist';
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
  mode: CommandMode;
  onModeChange: (mode: CommandMode) => void;
}

const BUCKETS: CommandBucket[] = ['Components', 'Boards', 'Examples', 'Actions'];

export function CommandPalette({ open, onClose, items, mode, onModeChange }: CommandPaletteProps) {
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
            <Command shouldFilter={mode === 'search'} loop>
              <div className="h-14 px-5 flex items-center gap-3 border-b border-border">
                <span className="text-magenta text-lg" aria-hidden>
                  {mode === 'assist' ? '✨' : '⌘'}
                </span>
                <Command.Input
                  autoFocus
                  placeholder={
                    mode === 'assist'
                      ? "Describe a change to your circuit, e.g. 'add an LED on PA5'"
                      : 'Search components, boards, examples…'
                  }
                  className="flex-1 bg-transparent outline-none text-[15px] placeholder:text-fg-tertiary"
                  onKeyDown={(event) => {
                    const value = (event.currentTarget as HTMLInputElement).value;
                    if (event.key === '/' && value === '' && mode === 'search') {
                      event.preventDefault();
                      onModeChange('assist');
                    } else if (event.key === 'Tab' && value === '') {
                      event.preventDefault();
                      onModeChange(mode === 'search' ? 'assist' : 'search');
                    } else if (event.key === 'Backspace' && value === '' && mode === 'assist') {
                      event.preventDefault();
                      onModeChange('search');
                    }
                  }}
                />
              </div>
              <Command.List className="max-h-[60vh] overflow-y-auto py-2">
                {mode === 'assist' ? (
                  <div className="px-5 py-6 text-fg-secondary text-center">
                    <p className="mb-2">AI assist is coming soon.</p>
                    <a className="text-accent hover:underline" href="mailto:hello@labwired.com?subject=AI%20assist%20waitlist">
                      Get notified
                    </a>
                  </div>
                ) : (
                  <>
                    <Command.Empty className="px-5 py-6 text-fg-tertiary text-center text-sm">
                      No matches.
                    </Command.Empty>
                    {BUCKETS.map((bucket) => {
                      const inBucket = items.filter((item) => item.bucket === bucket);
                      if (inBucket.length === 0) return null;
                      return (
                        <Command.Group key={bucket} heading={bucket} className="text-fg-tertiary text-[10px] uppercase tracking-wider px-3 py-1">
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
                  </>
                )}
              </Command.List>
            </Command>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
