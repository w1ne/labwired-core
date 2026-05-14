import type { Config } from 'tailwindcss';
import forms from '@tailwindcss/forms';

const config: Config = {
  content: [
    './index.html',
    './src/**/*.{ts,tsx}',
    '../ui/src/**/*.{ts,tsx}',
  ],
  theme: {
    extend: {
      colors: {
        bg: {
          base: 'var(--lw-bg-base)',
          surface: 'var(--lw-bg-surface)',
          elevated: 'var(--lw-bg-elevated)',
          canvas: 'var(--lw-bg-canvas)',
        },
        fg: {
          primary: 'var(--lw-fg-primary)',
          secondary: 'var(--lw-fg-secondary)',
          tertiary: 'var(--lw-fg-tertiary)',
        },
        border: {
          DEFAULT: 'var(--lw-border)',
          strong: 'var(--lw-border-strong)',
        },
        accent: {
          DEFAULT: 'var(--lw-accent)',
          hover: 'var(--lw-accent-hover)',
          soft: 'var(--lw-accent-soft)',
        },
        magenta: {
          DEFAULT: 'var(--lw-magenta)',
          soft: 'var(--lw-magenta-soft)',
        },
        ok: 'var(--lw-success)',
        warn: 'var(--lw-warning)',
        danger: 'var(--lw-danger)',
      },
      fontFamily: {
        sans: ['var(--lw-font-ui)'],
        mono: ['var(--lw-font-mono)'],
      },
      borderRadius: {
        card: 'var(--lw-radius-card)',
        button: 'var(--lw-radius-button)',
        pill: 'var(--lw-radius-pill)',
      },
      boxShadow: {
        glass: 'var(--lw-shadow-glass)',
      },
      transitionTimingFunction: {
        out: 'var(--lw-ease-out)',
      },
      transitionDuration: {
        micro: 'var(--lw-dur-micro)',
        panel: 'var(--lw-dur-panel)',
        modal: 'var(--lw-dur-modal)',
      },
    },
  },
  plugins: [forms],
};

export default config;
