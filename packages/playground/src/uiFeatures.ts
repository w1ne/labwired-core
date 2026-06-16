// Unified UI feature flags. There is ONE playground UI; which chrome features
// are shown is selectable per-context via URL params, instead of forking a
// separate "embed" UI. The embed (ChatGPT app) is just a preset: its
// inline_frame_url sets ?embed=1, which flips some defaults off — but every
// feature stays individually overridable (e.g. ?embed=1&menu=1 forces the menu
// back on). Desktop (no ?embed) gets everything on, including the frosted glass.
import { isEmbedMode } from '@labwired/ui';

export interface UiFeatures {
  /** Top-bar hamburger menu + nav drawer (projects, nav links, "open on laptop"). */
  menu: boolean;
  /** Frosted-glass backdrop-blur on chrome. Looks good full-size, hazy in a small embed. */
  glass: boolean;
}

/** A flag is on unless its param is explicitly "0" / "false". */
function flag(params: URLSearchParams, key: string, dflt: boolean): boolean {
  const v = params.get(key);
  if (v === null) return dflt;
  return v !== '0' && v !== 'false';
}

export function resolveUiFeatures(search?: string): UiFeatures {
  const params = new URLSearchParams(
    search ?? (typeof window !== 'undefined' ? window.location.search : ''),
  );
  const embed = isEmbedMode();
  // Embed preset turns menu + glass off by default; both remain overridable.
  return {
    menu: flag(params, 'menu', !embed),
    glass: flag(params, 'glass', !embed),
  };
}
