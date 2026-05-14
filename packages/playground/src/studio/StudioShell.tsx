import type { ReactNode } from 'react';
import { TopChrome } from './TopChrome';
import { useStudioLayout } from './useStudioLayout';

export interface StudioShellProps {
  boardName?: string;
  onShare?: () => void;
  children?: ReactNode;
}

export function StudioShell({ boardName = 'Untitled', onShare, children }: StudioShellProps) {
  const layout = useStudioLayout();

  return (
    <div className="relative w-full h-screen overflow-hidden bg-bg-base text-fg-primary">
      <TopChrome
        boardName={boardName}
        devMode={layout.devMode}
        onOpenCommand={layout.openCommand}
        onToggleDev={layout.toggleDev}
        onShare={onShare}
      />
      <main role="main" aria-label="Canvas" className="absolute inset-0 pt-11 bg-bg-canvas">
        {children}
      </main>
    </div>
  );
}
