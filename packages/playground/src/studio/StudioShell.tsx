import { useState, useEffect, type ReactNode } from 'react';
import { TopChrome } from './TopChrome';
import { HeroPrompt } from './HeroPrompt';
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
      <PaletteDrawer
        components={paletteComponents}
        open={layout.paletteOpen}
        onOpenChange={layout.setPaletteOpen}
        onDragStart={(type) => onPaletteDrag?.(type)}
      />
      <main role="main" aria-label="Canvas" className="absolute inset-0 pt-11 bg-bg-canvas">
        {children}
        {inspector}
        {isEmpty && (
          <div className="absolute inset-0 flex flex-col items-center justify-start pt-[32vh] gap-6 px-4 pointer-events-none">
            <div className="pointer-events-auto w-full max-w-[640px]">
              <HeroPrompt onFocus={layout.openCommand} />
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
