// Global (simulation-wide) chrome that lives OUTSIDE the canvas:
//   - TopChrome: LabWired branding, board-name pill, command palette
//     button, Upload, Code toggle, Library/For CI/GitHub links,
//     project + auth slots.
//   - PaletteDrawer: component palette (chip-agnostic).
//   - CommandPalette: ⌘K search (simulation-wide).
//   - Toast.
//
// Per-chip controls (board canvas, sim run/pause, registers,
// peripherals, dev-drawer tabs) live in <ChipBody> inside each
// active chip-shape on the canvas.
import { useEffect, type ReactNode } from 'react';
import { TopChrome } from './TopChrome';
import { PaletteDrawer, type PaletteComponent } from './PaletteDrawer';
import { Toast } from './Toast';
import { useStudioLayout } from './useStudioLayout';

export interface GlobalChromeProps {
  boardName?: string;
  onShare?: () => void;
  onUploadFirmware?: (file: File) => void;
  paletteComponents?: PaletteComponent[];
  onPaletteDrag?: (componentType: string) => void;
  authSlot?: ReactNode;
  projectSlot?: ReactNode;
  renderCommandPalette?: (
    commandOpen: boolean,
    closeCommand: () => void,
    openCommand: () => void,
  ) => ReactNode;
  onMountCommandRef?: (refs: { open: () => void; close: () => void }) => void;
  toast?: string | null;
  onDismissToast?: () => void;
}

export function GlobalChrome({
  boardName = 'Untitled',
  onShare,
  onUploadFirmware,
  paletteComponents = [],
  onPaletteDrag,
  authSlot,
  projectSlot,
  renderCommandPalette,
  onMountCommandRef,
  toast,
  onDismissToast,
}: GlobalChromeProps) {
  const layout = useStudioLayout();

  useEffect(() => {
    onMountCommandRef?.({ open: layout.openCommand, close: layout.closeCommand });
  }, [onMountCommandRef, layout.openCommand, layout.closeCommand]);

  return (
    <>
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
      {renderCommandPalette?.(layout.commandOpen, layout.closeCommand, layout.openCommand)}
      <Toast message={toast ?? null} onDismiss={() => onDismissToast?.()} />
    </>
  );
}

/// Height in pixels of the fixed TopChrome strip — exposed so the
/// canvas can offset its top edge.
export const GLOBAL_CHROME_HEIGHT = 44;
