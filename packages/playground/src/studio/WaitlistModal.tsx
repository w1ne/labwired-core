import { useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';

export interface WaitlistModalProps {
  open: boolean;
  labName: string;
  onClose: () => void;
}

export function WaitlistModal({ open, labName, onClose }: WaitlistModalProps) {
  const [email, setEmail] = useState('');

  useEffect(() => {
    if (!open) return;
    const handleEsc = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleEsc);
    return () => document.removeEventListener('keydown', handleEsc);
  }, [open, onClose]);

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.16 }}
          className="fixed inset-0 z-50 flex items-center justify-center bg-bg-base/60 backdrop-blur-sm"
          onClick={onClose}
        >
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-label={`${labName} waitlist`}
            initial={{ opacity: 0, scale: 0.96 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.96 }}
            transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
            className="lw-glass w-[440px] p-6"
            onClick={(event) => event.stopPropagation()}
          >
            <h2 className="text-fg-primary text-base font-semibold mb-2">{labName} is coming soon</h2>
            <p className="text-fg-secondary mb-5">
              This lab arrives with our Device Library Phase 1. Drop your email to get the launch ping.
            </p>
            <form
              onSubmit={(event) => {
                event.preventDefault();
                if (!email) return;
                window.location.href = `mailto:hello@labwired.com?subject=Waitlist:%20${encodeURIComponent(labName)}&body=Sign%20me%20up:%20${encodeURIComponent(email)}`;
                onClose();
              }}
              className="flex gap-2"
            >
              <input
                type="email"
                required
                value={email}
                onChange={(event) => setEmail(event.target.value)}
                placeholder="you@example.com"
                className="flex-1 h-9 px-3 rounded-button bg-bg-surface border border-border text-fg-primary outline-none focus:border-accent"
              />
              <button
                type="submit"
                className="h-9 px-4 rounded-button bg-accent text-bg-base font-medium hover:bg-accent-hover transition-colors duration-micro"
              >
                Notify me
              </button>
            </form>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
