import { useRef } from 'react';
import clsx from 'clsx';

export interface TopChromeProps {
  boardName: string;
  devMode: boolean;
  onOpenCommand: () => void;
  onToggleDev: () => void;
  onShare?: () => void;
  onUploadFirmware?: (file: File) => void;
}

export function TopChrome({ boardName, devMode, onOpenCommand, onToggleDev, onShare, onUploadFirmware }: TopChromeProps) {
  const uploadInputRef = useRef<HTMLInputElement>(null);
  return (
    <header
      role="banner"
      className="absolute top-0 inset-x-0 z-30 flex items-center gap-3 h-11 px-3 bg-[rgba(13,14,18,0.6)] backdrop-blur"
    >
      <a href="/" className="flex items-center gap-2 text-fg-primary font-semibold tracking-tight shrink-0">
        <svg viewBox="0 0 20 20" width="18" height="18" aria-hidden="true">
          <path d="M11 2 4 12h4l-1 6 8-10h-4l1-6z" fill="currentColor" />
        </svg>
        LabWired
      </a>
      <span className="text-fg-tertiary text-[11px] hidden lg:inline tracking-[0.01em] shrink-0">
        Deterministic firmware simulation, no hardware needed
      </span>
      <span className="text-fg-tertiary shrink-0" aria-hidden>›</span>
      <span className="text-fg-secondary truncate max-w-[24ch]">{boardName}</span>

      <div className="flex-1 max-w-[440px] mx-auto">
        <button
          type="button"
          onClick={onOpenCommand}
          style={{ borderRadius: 999 }}
          className="w-full h-8 px-4 flex items-center gap-2 bg-white/[0.05] hover:bg-white/[0.09] text-left transition-colors duration-micro outline-none border-0 focus-visible:ring-2 focus-visible:ring-accent/50"
          aria-label="Open command palette"
        >
          <span aria-hidden className="text-fg-tertiary text-[11px] font-mono">⌘K</span>
          <span className="flex-1 text-fg-tertiary text-[12px] truncate">Search components, boards, examples…</span>
        </button>
      </div>

      {onUploadFirmware && (
        <>
          <input
            ref={uploadInputRef}
            type="file"
            accept=".elf,.bin,.hex,.uf2,application/octet-stream"
            className="hidden"
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) onUploadFirmware(file);
              event.target.value = '';
            }}
          />
          <button
            type="button"
            onClick={() => uploadInputRef.current?.click()}
            aria-label="Upload firmware ELF"
            title="Upload your compiled firmware (.elf / .bin / .hex)"
            className="h-7 px-3 rounded-pill text-xs font-medium bg-white/[0.05] text-fg-secondary hover:bg-white/[0.10] hover:text-fg-primary transition-colors duration-micro border-0 outline-none focus-visible:ring-2 focus-visible:ring-accent/50 flex items-center gap-1.5 shrink-0"
          >
            <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
              <path d="M8 2v9M5 5l3-3 3 3M3 12v2h10v-2" />
            </svg>
            Upload
          </button>
        </>
      )}
      <button
        type="button"
        role="switch"
        aria-checked={devMode}
        aria-label="Dev mode"
        onClick={onToggleDev}
        className={clsx(
          'h-7 px-3 rounded-pill text-xs font-medium transition-colors duration-micro shrink-0',
          devMode
            ? 'bg-magenta-soft text-magenta border border-magenta/40'
            : 'bg-bg-surface/60 text-fg-secondary border border-border hover:text-fg-primary'
        )}
      >
        Dev {devMode ? 'on' : 'off'}
      </button>
      <a
        href="../gallery.html"
        className="hidden lg:flex h-7 px-3 rounded-pill text-xs font-medium text-fg-secondary hover:text-fg-primary hover:bg-white/[0.05] transition-colors duration-micro items-center shrink-0"
        title="Browse featured labs"
      >
        Gallery
      </a>
      <a
        href="../ci.html"
        className="hidden md:flex h-7 px-3 rounded-pill text-xs font-medium bg-magenta-soft text-magenta hover:bg-magenta/20 border border-magenta/30 transition-colors duration-micro items-center gap-1.5 shrink-0"
        title="Use LabWired in your CI pipeline"
      >
        <svg viewBox="0 0 16 16" width="11" height="11" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
          <path d="M2 6h12M2 10h12M5 2v12M11 2v12" />
        </svg>
        For CI
      </a>
      <button
        type="button"
        onClick={onShare}
        className="h-7 px-3 rounded-pill text-xs font-medium bg-accent text-bg-base hover:bg-accent-hover transition-colors duration-micro shrink-0"
      >
        Share
      </button>
    </header>
  );
}
