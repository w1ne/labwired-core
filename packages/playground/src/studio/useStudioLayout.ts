import { useCallback, useState } from 'react';

const DEV_KEY = 'labwired:dev-mode';

export interface StudioLayoutState {
  paletteOpen: boolean;
  commandOpen: boolean;
  devMode: boolean;
}

export interface StudioLayoutActions {
  setPaletteOpen: (open: boolean) => void;
  openCommand: () => void;
  closeCommand: () => void;
  toggleDev: () => void;
}

export function useStudioLayout(): StudioLayoutState & StudioLayoutActions {
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [commandOpen, setCommandOpen] = useState(false);
  const [devMode, setDevMode] = useState(() => {
    if (typeof window === 'undefined') return false;
    return localStorage.getItem(DEV_KEY) === '1';
  });

  const openCommand = useCallback(() => setCommandOpen(true), []);
  const closeCommand = useCallback(() => setCommandOpen(false), []);
  const toggleDev = useCallback(() => {
    setDevMode((current) => {
      const next = !current;
      try {
        localStorage.setItem(DEV_KEY, next ? '1' : '0');
      } catch { /* localStorage may be unavailable */ }
      return next;
    });
  }, []);

  return { paletteOpen, commandOpen, devMode, setPaletteOpen, openCommand, closeCommand, toggleDev };
}
