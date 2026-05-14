import { useState, useEffect, type ReactNode } from 'react';
import { TopChrome } from './TopChrome';
import { ChipRow } from './ChipRow';
import { WaitlistModal } from './WaitlistModal';
import { PaletteDrawer, type PaletteComponent } from './PaletteDrawer';
import { useStudioLayout } from './useStudioLayout';

export interface StudioShellProps {
  boardName?: string;
  isEmpty?: boolean;
  onShare?: () => void;
  onPickLab?: (labId: string) => void;
  paletteComponents?: PaletteComponent[];
  onPaletteDrag?: (componentType: string) => void;
  inspector?: ReactNode;
  simDock?: ReactNode;
  renderDevDrawer?: (devMode: boolean) => ReactNode;
  renderCommandPalette?: (commandOpen: boolean, closeCommand: () => void, openCommand: () => void) => ReactNode;
  onMountCommandRef?: (refs: { open: () => void; close: () => void }) => void;
  children?: ReactNode;
}

const LOCKED_NAMES: Record<string, string> = {
  'bme280-weather': 'BME280 Weather',
  'oled-hello': 'OLED Hello',
  'gps-trail': 'GPS Trail',
  'tft-demo': 'TFT Demo',
};

export function StudioShell({
  boardName = 'Untitled',
  isEmpty = false,
  onShare,
  onPickLab,
  paletteComponents = [],
  onPaletteDrag,
  inspector,
  simDock,
  renderDevDrawer,
  renderCommandPalette,
  onMountCommandRef,
  children,
}: StudioShellProps) {
  const layout = useStudioLayout();
  const [waitlistLab, setWaitlistLab] = useState<string | null>(null);

  useEffect(() => {
    onMountCommandRef?.({ open: layout.openCommand, close: layout.closeCommand });
  }, [onMountCommandRef, layout.openCommand, layout.closeCommand]);

  return (
    <div className="relative w-full h-screen overflow-hidden bg-bg-base text-fg-primary">
      <TopChrome
        boardName={boardName}
        devMode={layout.devMode}
        onOpenCommand={layout.openCommand}
        onToggleDev={layout.toggleDev}
        onShare={onShare}
      />
      {layout.mobile && (
        <div className="absolute top-11 inset-x-0 z-30 bg-warning/10 border-b border-warning/30 px-4 py-2 text-warning text-[12px] text-center">
          View only on mobile — open on desktop to edit.
        </div>
      )}
      {!layout.mobile && (
        <PaletteDrawer
          components={paletteComponents}
          open={layout.paletteOpen}
          onOpenChange={layout.setPaletteOpen}
          onDragStart={(type) => onPaletteDrag?.(type)}
        />
      )}
      <main role="main" aria-label="Canvas" className="absolute inset-0 pt-11 bg-bg-canvas">
        {children}
        {inspector}
        {isEmpty && (
          <div className="absolute inset-0 flex flex-col items-center justify-start pt-[28vh] gap-5 px-4 pointer-events-none">
            <div className="pointer-events-none text-center">
              <h2 className="text-fg-primary text-xl font-semibold tracking-tight">Pick a starter to begin</h2>
              <p className="text-fg-tertiary text-[13px] mt-1">
                Or press <kbd className="px-1.5 py-0.5 rounded border border-border text-fg-secondary font-mono text-[11px]">⌘K</kbd> to search components & boards
              </p>
            </div>
            <div className="pointer-events-auto">
              <ChipRow
                onPick={(labId) => onPickLab?.(labId)}
                onLocked={(labId) => setWaitlistLab(LOCKED_NAMES[labId] ?? labId)}
              />
            </div>
          </div>
        )}
        {simDock}
        {renderDevDrawer?.(layout.devMode)}
      </main>
      <WaitlistModal
        open={!!waitlistLab}
        labName={waitlistLab ?? ''}
        onClose={() => setWaitlistLab(null)}
      />
      {renderCommandPalette?.(layout.commandOpen, layout.closeCommand, layout.openCommand)}
    </div>
  );
}
