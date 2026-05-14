import clsx from 'clsx';

export interface TopChromeProps {
  boardName: string;
  devMode: boolean;
  onOpenCommand: () => void;
  onToggleDev: () => void;
  onShare?: () => void;
}

export function TopChrome({ boardName, devMode, onOpenCommand, onToggleDev, onShare }: TopChromeProps) {
  return (
    <header
      role="banner"
      className="absolute top-0 inset-x-0 z-30 flex items-center gap-3 h-11 px-3 bg-[rgba(13,14,18,0.6)] backdrop-blur"
    >
      <a href="/" className="flex items-center gap-2 text-fg-primary font-semibold tracking-tight">
        <svg viewBox="0 0 20 20" width="18" height="18" aria-hidden="true">
          <path d="M11 2 4 12h4l-1 6 8-10h-4l1-6z" fill="currentColor" />
        </svg>
        LabWired
      </a>
      <span className="text-fg-tertiary" aria-hidden>›</span>
      <span className="text-fg-secondary truncate max-w-[28ch]">{boardName}</span>

      <div className="flex-1 max-w-[520px] mx-auto">
        <button
          type="button"
          onClick={onOpenCommand}
          className="w-full h-8 px-3 flex items-center gap-2 rounded-button bg-bg-surface/70 border border-border text-fg-tertiary text-left hover:border-border-strong transition-colors duration-micro"
        >
          <span aria-hidden>⌘K</span>
          <input
            tabIndex={-1}
            readOnly
            placeholder="Search components, boards, examples…"
            onClick={onOpenCommand}
            className="bg-transparent flex-1 outline-none text-fg-secondary placeholder:text-fg-tertiary cursor-pointer"
          />
        </button>
      </div>

      <button
        type="button"
        role="switch"
        aria-checked={devMode}
        aria-label="Dev mode"
        onClick={onToggleDev}
        className={clsx(
          'h-7 px-3 rounded-pill text-xs font-medium transition-colors duration-micro',
          devMode
            ? 'bg-magenta-soft text-magenta border border-magenta/40'
            : 'bg-bg-surface/60 text-fg-secondary border border-border hover:text-fg-primary'
        )}
      >
        Dev {devMode ? 'on' : 'off'}
      </button>
      <button
        type="button"
        onClick={onShare}
        className="h-7 px-3 rounded-pill text-xs font-medium bg-accent text-bg-base hover:bg-accent-hover transition-colors duration-micro"
      >
        Share
      </button>
    </header>
  );
}
