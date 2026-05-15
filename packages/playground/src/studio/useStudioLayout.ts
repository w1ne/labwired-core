import { useCallback, useEffect, useState } from 'react';

const DEV_KEY = 'labwired:dev-mode';

export const MOBILE_BREAKPOINT = 768;

export function isMobileViewport(widthPx: number = typeof window === 'undefined' ? 1440 : window.innerWidth): boolean {
  return widthPx < MOBILE_BREAKPOINT;
}

export interface StudioLayoutState {
  paletteOpen: boolean;
  commandOpen: boolean;
  devMode: boolean;
  mobile: boolean;
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
  const [mobile, setMobile] = useState(() => isMobileViewport());

  useEffect(() => {
    const onResize = () => setMobile(isMobileViewport());
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

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

  return { paletteOpen, commandOpen, devMode, mobile, setPaletteOpen, openCommand, closeCommand, toggleDev };
}
