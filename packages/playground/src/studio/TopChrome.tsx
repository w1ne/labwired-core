import { useRef, type ReactNode } from 'react';
import { GlobalLogo, GlobalNav } from '../components/GlobalNav';
import { ToolsMenu, type ToolItem } from './ToolsMenu';

export interface TopChromeProps {
  boardName: string;
  onOpenCommand: () => void;
  onShare?: () => void;
  onUploadFirmware?: (file: File) => void;
  onOpenTools?: () => void;
  tools?: ToolItem[];
  authSlot?: ReactNode;
  projectSlot?: ReactNode;
}

export function TopChrome({ boardName, onOpenCommand, onShare, onUploadFirmware, onOpenTools, tools = [], authSlot, projectSlot }: TopChromeProps) {
  const uploadInputRef = useRef<HTMLInputElement>(null);
  const navToolTriggerClass =
    'flex h-7 px-3 rounded-pill text-xs font-medium transition-colors duration-150 items-center shrink-0 text-fg-secondary hover:text-fg-primary hover:bg-white/[0.05] border-0 bg-transparent appearance-none';
  return (
    <header
      role="banner"
      className="absolute top-0 inset-x-0 z-30 flex items-center gap-2 sm:gap-3 h-8 px-2 sm:px-3 bg-[rgba(13,14,18,0.6)] backdrop-blur overflow-visible"
    >
      <GlobalLogo variant="dark" />
      <span
        title="LabWired runs your firmware deterministically — the same .elf produces the same output every run. Drop it into CI for regression tests."
        className="hidden md:inline-flex items-center gap-1.5 h-5 px-2 rounded-pill bg-success/10 border border-success/30 text-success text-[10.5px] font-medium tracking-[0.02em] shrink-0"
      >
        <span aria-hidden className="w-1.5 h-1.5 rounded-full bg-success shadow-[0_0_6px_rgba(61,214,140,0.7)]" />
        <span className="hidden xl:inline">Deterministic</span>
        <span aria-hidden className="text-success/40 hidden xl:inline">·</span>
        Cycle-accurate
      </span>
      <span className="text-fg-tertiary shrink-0 hidden sm:inline" aria-hidden>›</span>
      <span className="text-fg-secondary truncate max-w-[14ch] sm:max-w-[24ch]">{boardName}</span>

      <div className="flex-1 max-w-[360px] mx-auto min-w-0 hidden md:block">
        <button
          type="button"
          onClick={onOpenCommand}
          style={{ borderRadius: 999 }}
          className="w-full h-6 px-4 flex items-center gap-2 bg-white/[0.05] hover:bg-white/[0.09] text-left transition-colors duration-micro outline-none border-0 focus-visible:ring-2 focus-visible:ring-accent/50"
          aria-label="Search components, boards, and examples"
        >
          <span aria-hidden className="text-fg-tertiary text-[11px] font-mono">⌘K</span>
          <span className="flex-1 text-fg-tertiary text-[12px] truncate">Search components, boards, examples…</span>
        </button>
      </div>

      {/* Mobile spacer — when search box is hidden, push the controls to the right edge. */}
      <div className="flex-1 md:hidden" />

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
            className="hidden sm:flex h-6 px-3 rounded-pill text-xs font-medium bg-white/[0.05] text-fg-secondary hover:bg-white/[0.10] hover:text-fg-primary transition-colors duration-micro border-0 outline-none focus-visible:ring-2 focus-visible:ring-accent/50 items-center gap-1.5 shrink-0"
          >
            <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
              <path d="M8 2v9M5 5l3-3 3 3M3 12v2h10v-2" />
            </svg>
            Upload
          </button>
        </>
      )}
      <div className="hidden sm:flex items-center gap-1">
        <GlobalNav
          active="playground"
          variant="dark"
          exclude={['validation']}
          onToolsClick={onOpenTools}
          toolsSlot={
            tools.length > 0 ? (
              <ToolsMenu
                tools={tools}
                triggerClassName={navToolTriggerClass}
                showIcon={false}
                showCaret={false}
              />
            ) : undefined
          }
        />
      </div>
      <div className="hidden sm:contents">{projectSlot}</div>
      {authSlot}
      <button
        type="button"
        onClick={onShare}
        className="hidden sm:flex h-6 px-3 rounded-pill text-xs font-medium bg-accent text-bg-base hover:bg-accent-hover transition-colors duration-micro shrink-0 items-center"
      >
        Share
      </button>
      {/* Mobile-only command palette opener (replaces the wide ⌘K search box) */}
      <button
        type="button"
        onClick={onOpenCommand}
        aria-label="Open command palette"
        className="md:hidden flex items-center justify-center h-7 w-7 rounded-pill bg-white/[0.05] text-fg-secondary hover:bg-white/[0.10] shrink-0"
      >
        <svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
          <circle cx="7" cy="7" r="4.5" />
          <path d="m10.5 10.5 3 3" />
        </svg>
      </button>
    </header>
  );
}
