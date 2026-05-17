import { useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';

export interface ToastProps {
  message: string | null;
  onDismiss: () => void;
  durationMs?: number;
}

export function Toast({ message, onDismiss, durationMs = 4000 }: ToastProps) {
  useEffect(() => {
    if (!message) return;
    const timer = setTimeout(onDismiss, durationMs);
    return () => clearTimeout(timer);
  }, [message, durationMs, onDismiss]);

  const isError = !!message && /(failed|error|cannot|unable)/i.test(message);

  return (
    <AnimatePresence>
      {message && (
        <motion.div
          role="status"
          aria-live="polite"
          initial={{ opacity: 0, y: 12, scale: 0.96 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: 12, scale: 0.96 }}
          transition={{ duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
          className="lw-glass fixed bottom-20 left-1/2 -translate-x-1/2 z-40 h-10 px-4 flex items-center gap-2 text-fg-primary text-[13px] font-medium shrink-0"
        >
          <span className={isError ? 'text-danger' : 'text-ok'} aria-hidden>
            {isError ? '⚠' : '✓'}
          </span>
          {message}
        </motion.div>
      )}
    </AnimatePresence>
  );
}
