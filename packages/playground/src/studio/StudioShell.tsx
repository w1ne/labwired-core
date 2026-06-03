import { useEffect, type ReactNode } from 'react';
import { TopChrome } from './TopChrome';
import { ChipRow } from './ChipRow';
import { PaletteDrawer, type PaletteComponent } from './PaletteDrawer';
import { Toast } from './Toast';
import type { ToolItem } from './ToolsMenu';
import { useStudioLayout } from './useStudioLayout';

export interface StudioShellProps {
  boardName?: string;
  isEmpty?: boolean;
  onShare?: () => void;
  onPickLab?: (labId: string) => void;
  onUploadFirmware?: (file: File) => void;
  onOpenTools?: () => void;
  tools?: ToolItem[];
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
  /**
   * Host-controlled "dev mode" (code editor open). Drives the mobile dev
   * drawer and the sim-dock offset. The on-canvas toggle was removed, so this
   * is currently always closed on desktop.
   */
  devMode?: boolean;
  children?: ReactNode;
}

export function StudioShell({
  boardName = 'Untitled',
  isEmpty = false,
  onShare,
  onPickLab,
  onUploadFirmware,
  onOpenTools,
  tools,
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
  devMode: devModeProp,
  children,
}: StudioShellProps) {
  const layout = useStudioLayout();
  const devMode = devModeProp ?? layout.devMode;

  useEffect(() => {
    onMountCommandRef?.({ open: layout.openCommand, close: layout.closeCommand });
  }, [onMountCommandRef, layout.openCommand, layout.closeCommand]);

  return (
    <div className="relative w-full h-screen overflow-hidden bg-bg-base text-fg-primary">
      <TopChrome
        boardName={boardName}
        onOpenCommand={layout.openCommand}
        onShare={onShare}
        onUploadFirmware={onUploadFirmware}
        onOpenTools={onOpenTools}
        tools={tools}
        authSlot={authSlot}
        projectSlot={projectSlot}
      />
      <PaletteDrawer
        components={paletteComponents}
        open={layout.paletteOpen}
        onOpenChange={layout.setPaletteOpen}
        onDragStart={(type) => onPaletteDrag?.(type)}
      />
      <main role="main" aria-label="Canvas" className="absolute inset-0 pt-8 bg-bg-canvas">
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
                onLocked={() => { /* no locked labs in v1 */ }}
              />
            </div>
          </div>
        )}
        {simDock && (
          <div
            className="absolute left-1/2 -translate-x-1/2 z-20 transition-[bottom] duration-panel ease-out"
            style={{ bottom: devMode ? 256 : 16 }}
          >
            {simDock}
          </div>
        )}
        {renderDevDrawer?.(devMode, !layout.mobile && layout.paletteOpen ? 280 : 0)}
      </main>
      {renderCommandPalette?.(layout.commandOpen, layout.closeCommand, layout.openCommand)}
      <Toast message={toast ?? null} onDismiss={() => onDismissToast?.()} />
    </div>
  );
}
