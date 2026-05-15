import { useEffect } from 'react';
import { UserProfile } from '@clerk/clerk-react';

export interface AccountPanelProps {
  open: boolean;
  onClose: () => void;
}

export function AccountPanel({ open, onClose }: AccountPanelProps) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Account"
      className="fixed inset-0 z-50 flex items-start justify-center p-4 sm:p-8 overflow-auto"
    >
      <button
        type="button"
        aria-label="Close account panel"
        onClick={onClose}
        className="absolute inset-0 bg-black/70 backdrop-blur-sm border-0 outline-none"
      />
      <div className="relative flex flex-col items-stretch gap-3 w-full max-w-[880px]">
        <div className="flex justify-end">
          <button
            type="button"
            onClick={onClose}
            className="h-8 px-3 rounded-md text-xs font-medium bg-white/[0.08] text-fg-secondary hover:text-fg-primary hover:bg-white/[0.12] border-0"
          >
            Close
          </button>
        </div>
        <UserProfile routing="hash" />
      </div>
    </div>
  );
}
