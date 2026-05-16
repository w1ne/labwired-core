import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { ClerkProvider } from '@clerk/clerk-react';
import '../styles/tokens.css';
import '../styles/tailwind.css';
import './ci-light.css';
import { CiLanding } from './CiLanding';
import { CLERK_PUBLISHABLE_KEY } from '../clerk';

// CiLanding calls useUser() to build a Stripe upgrade URL prefilled with the
// Clerk user_id + email when signed in. Without ClerkProvider, the hook
// throws on mount and the page crashes to an empty root → black screen.
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ClerkProvider publishableKey={CLERK_PUBLISHABLE_KEY} afterSignOutUrl="/">
      <CiLanding />
    </ClerkProvider>
  </StrictMode>,
);
