import type { Appearance } from '@clerk/shared/types';

// Mirrors tokens.css. Clerk's CSS-in-JS layer does not always resolve CSS custom
// properties, so values are inlined.
export const clerkAppearance: Appearance = {
  variables: {
    colorPrimary: '#5B9DFF',
    colorBackground: '#13151B',
    colorText: '#F2F4F9',
    colorTextSecondary: '#9098A8',
    colorTextOnPrimaryBackground: '#0A0B0F',
    colorInputBackground: '#0E1015',
    colorInputText: '#F2F4F9',
    colorNeutral: '#9098A8',
    colorDanger: '#F2545B',
    colorSuccess: '#3DD68C',
    colorWarning: '#F5B642',
    fontFamily:
      "'Inter', system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
    fontFamilyButtons:
      "'Inter', system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
    borderRadius: '8px',
  },
  elements: {
    card: {
      backgroundColor: '#13151B',
      border: '1px solid #262A33',
      boxShadow:
        'inset 0 1px 0 rgba(255,255,255,0.04), 0 24px 48px -16px rgba(0,0,0,0.48)',
    },
    headerTitle: { color: '#F2F4F9', fontWeight: 600 },
    headerSubtitle: { color: '#9098A8' },
    socialButtonsIconButton: {
      backgroundColor: 'rgba(255,255,255,0.05)',
      border: '1px solid #262A33',
      '&:hover': { backgroundColor: 'rgba(255,255,255,0.09)' },
    },
    socialButtonsBlockButton: {
      backgroundColor: 'rgba(255,255,255,0.05)',
      border: '1px solid #262A33',
      '&:hover': { backgroundColor: 'rgba(255,255,255,0.09)' },
    },
    dividerLine: { backgroundColor: '#262A33' },
    dividerText: { color: '#5A6178' },
    formFieldLabel: { color: '#9098A8', fontSize: '12px' },
    formFieldInput: {
      backgroundColor: '#0E1015',
      border: '1px solid #262A33',
      color: '#F2F4F9',
      '&:focus': { borderColor: '#5B9DFF', boxShadow: '0 0 0 1px #5B9DFF' },
    },
    formButtonPrimary: {
      backgroundColor: '#5B9DFF',
      color: '#0A0B0F',
      fontWeight: 500,
      '&:hover': { backgroundColor: '#7DB1FF' },
      '&:focus': { boxShadow: '0 0 0 2px rgba(91,157,255,0.4)' },
    },
    footerActionLink: {
      color: '#5B9DFF',
      '&:hover': { color: '#7DB1FF' },
    },
    footer: {
      backgroundColor: 'transparent',
      borderTop: '1px solid #262A33',
    },
    userButtonPopoverCard: {
      backgroundColor: '#13151B',
      border: '1px solid #262A33',
    },
    userButtonPopoverActionButton: {
      color: '#F2F4F9',
      '&:hover': { backgroundColor: 'rgba(255,255,255,0.05)' },
    },
  },
};
