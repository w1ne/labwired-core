/**
 * Inline SVG icons for the playground toolbar.
 * Modeled after VS Code codicons — simple, 16x16, stroke-based.
 */

import type { CSSProperties } from 'react';

interface IconProps {
  size?: number;
  style?: CSSProperties;
  className?: string;
}

const defaults = { size: 16 };

export function CheckIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...rest}>
      <polyline points="3.5 8.5 6.5 11.5 12.5 4.5" />
    </svg>
  );
}

export function UploadIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...rest}>
      <line x1="8" y1="10" x2="8" y2="3" />
      <polyline points="5 5.5 8 2.5 11 5.5" />
      <path d="M3 10v2.5a1 1 0 001 1h8a1 1 0 001-1V10" />
    </svg>
  );
}

export function CodeIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...rest}>
      <polyline points="5.5 4 2 8 5.5 12" />
      <polyline points="10.5 4 14 8 10.5 12" />
    </svg>
  );
}

export function PanelBottomIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...rest}>
      <rect x="2" y="2" width="12" height="12" rx="1.5" />
      <line x1="2" y1="10" x2="14" y2="10" />
    </svg>
  );
}

export function ShareIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...rest}>
      <circle cx="4" cy="8" r="1.5" />
      <circle cx="12" cy="4" r="1.5" />
      <circle cx="12" cy="12" r="1.5" />
      <line x1="5.4" y1="7.2" x2="10.6" y2="4.8" />
      <line x1="5.4" y1="8.8" x2="10.6" y2="11.2" />
    </svg>
  );
}

export function ExportIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...rest}>
      <line x1="8" y1="2" x2="8" y2="9" />
      <polyline points="5 6.5 8 9.5 11 6.5" />
      <path d="M3 10v2.5a1 1 0 001 1h8a1 1 0 001-1V10" />
    </svg>
  );
}

export function ImportIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...rest}>
      <path d="M2.5 4.5h5l1.5 1.5h4.5v6.5a1 1 0 01-1 1h-10a1 1 0 01-1-1v-7a1 1 0 011-1z" />
    </svg>
  );
}

export function UndoIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...rest}>
      <polyline points="4 6 2 8 4 10" />
      <path d="M2 8h8a3 3 0 010 6H8" />
    </svg>
  );
}

export function RedoIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...rest}>
      <polyline points="12 6 14 8 12 10" />
      <path d="M14 8H6a3 3 0 000 6h2" />
    </svg>
  );
}

export function StopIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" {...rest}>
      <rect x="3.5" y="3.5" width="9" height="9" rx="1" />
    </svg>
  );
}

export function SidebarLeftIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...rest}>
      <rect x="2" y="2" width="12" height="12" rx="1.5" />
      <line x1="6" y1="2" x2="6" y2="14" />
    </svg>
  );
}

export function SidebarRightIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...rest}>
      <rect x="2" y="2" width="12" height="12" rx="1.5" />
      <line x1="10" y1="2" x2="10" y2="14" />
    </svg>
  );
}

export function ChevronLeftIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...rest}>
      <polyline points="10 3 5 8 10 13" />
    </svg>
  );
}

export function ChevronRightIcon({ size = defaults.size, ...rest }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...rest}>
      <polyline points="6 3 11 8 6 13" />
    </svg>
  );
}
