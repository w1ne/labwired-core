import { motion } from 'framer-motion';

export interface HeroPromptProps {
  onFocus: () => void;
}

export function HeroPrompt({ onFocus }: HeroPromptProps) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.32, ease: [0.16, 1, 0.3, 1] }}
      className="lw-glass w-[min(560px,calc(100vw-32px))] mx-auto"
    >
      <button
        type="button"
        onClick={onFocus}
        className="w-full h-14 px-5 flex items-center gap-3 text-left"
        aria-label="Open command palette"
      >
        <span className="text-magenta text-lg" aria-hidden>
          ✨
        </span>
        <input
          tabIndex={-1}
          readOnly
          onFocus={onFocus}
          onClick={onFocus}
          placeholder="Describe what to build, or pick a starter…"
          className="flex-1 bg-transparent outline-none text-[16px] placeholder:text-fg-tertiary cursor-pointer"
        />
        <kbd className="text-fg-tertiary text-xs border border-border rounded px-1.5 py-0.5">⌘K</kbd>
      </button>
    </motion.div>
  );
}
