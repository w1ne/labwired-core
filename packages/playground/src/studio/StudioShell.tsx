import { useEffect, type ReactNode } from 'react';
import { TopChrome } from './TopChrome';
import { ChipRow } from './ChipRow';
import { PaletteDrawer, type PaletteComponent } from './PaletteDrawer';
import { Toast } from './Toast';
import { useStudioLayout } from './useStudioLayout';

export interface StudioShellProps {
  boardName?: string;
  isEmpty?: boolean;
  onShare?: () => void;
  onPickLab?: (labId: string) => void;
  onUploadFirmware?: (file: File) => void;
  paletteComponents?: PaletteComponent[];
  onPaletteDrag?: (componentType: string) => void;
  inspector?: ReactNode;
  simDock?: ReactNode;
  authSlot?: ReactNode;
  projectSlot?: ReactNode;
  renderDevDrawer?: (devMode: boolean, leftOffset: number) => ReactNode;
  renderCommandPalette?: (commandOpen: boolean, closeCommand: () => void, openCommand: () => void) => ReactNode;
  onMountCommandRef?: (refs: { open: () => void; close: () => void }) => void;
  toast?: string | null;
  onDismissToast?: () => void;
  children?: ReactNode;
  /// 'integrated' (default, legacy) renders TopChrome + drawers +
  /// toast inside the shell. 'body-only' renders just the per-chip
  /// main area (used inside ChipInspectorWindow so the global menu
  /// can live outside the floating window).
  chromeMode?: 'integrated' | 'body-only';
}

export function StudioShell({
  boardName = 'Untitled',
  isEmpty = false,
  onShare,
  onPickLab,
  onUploadFirmware,
  paletteComponents = [],
  onPaletteDrag,
  inspector,
  simDock,
  authSlot,
  projectSlot,
  renderDevDrawer,
  renderCommandPalette,
  onMountCommandRef,
  toast,
  onDismissToast,
  children,
  chromeMode = 'integrated',
}: StudioShellProps) {
  const layout = useStudioLayout();

  useEffect(() => {
    onMountCommandRef?.({ open: layout.openCommand, close: layout.closeCommand });
  }, [onMountCommandRef, layout.openCommand, layout.closeCommand]);

  const mainArea = (
    <main role="main" aria-label="Canvas" className="absolute inset-0 bg-bg-canvas">
      {children}
      {inspector}
      {isEmpty && (
        <div className="absolute inset-0 flex flex-col items-center justify-start pt-[20vh] gap-5 px-4 pointer-events-none">
          <div className="pointer-events-none text-center">
            <h2 className="text-fg-primary text-xl font-semibold tracking-tight">Pick a starter to begin</h2>
            <p className="text-fg-tertiary text-[13px] mt-1">
              Or press <kbd className="px-1.5 py-0.5 rounded border border-border text-fg-secondary font-mono text-[11px]">⌘K</kbd> to search components & boards
            </p>
          </div>
          <div className="pointer-events-auto">
            <ChipRow
              onPick={(labId) => onPickLab?.(labId)}
              onLocked={() => { /* no locked labs in v1 */ }}
            />
          </div>
        </div>
      )}
      {simDock && (
        <div
          className="absolute left-1/2 -translate-x-1/2 z-20 transition-[bottom] duration-panel ease-out"
          style={{ bottom: layout.devMode ? 256 : 16 }}
        >
          {simDock}
        </div>
      )}
      {renderDevDrawer?.(layout.devMode, 0)}
    </main>
  );

  if (chromeMode === 'body-only') {
    // Body-only mode: render just the per-chip area (no TopChrome,
    // no PaletteDrawer, no command/toast). The global chrome is
    // rendered by <GlobalChrome> elsewhere in the tree. Wrap in a
    // relative container so the absolutely-positioned children sit
    // inside the chip-inspector window's bounds.
    return <div className="relative w-full h-full overflow-hidden bg-bg-base text-fg-primary">{mainArea}</div>;
  }

  return (
    <div className="relative w-full h-screen overflow-hidden bg-bg-base text-fg-primary">
      <TopChrome
        boardName={boardName}
        devMode={layout.devMode}
        onOpenCommand={layout.openCommand}
        onToggleDev={layout.toggleDev}
        onShare={onShare}
        onUploadFirmware={onUploadFirmware}
        authSlot={authSlot}
        projectSlot={projectSlot}
      />
      <PaletteDrawer
        components={paletteComponents}
        open={layout.paletteOpen}
        onOpenChange={layout.setPaletteOpen}
        onDragStart={(type) => onPaletteDrag?.(type)}
      />
      <div className="absolute inset-0 pt-11">{mainArea}</div>
      {renderCommandPalette?.(layout.commandOpen, layout.closeCommand, layout.openCommand)}
      <Toast message={toast ?? null} onDismiss={() => onDismissToast?.()} />
    </div>
  );
}
