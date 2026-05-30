const key = import.meta.env.VITE_CLERK_PUBLISHABLE_KEY as string | undefined;
const authDisabled = import.meta.env.VITE_DISABLE_AUTH === 'true';

if (!key && !authDisabled) {
  throw new Error(
    'VITE_CLERK_PUBLISHABLE_KEY is missing. Set it in packages/playground/.env.local — ' +
      'copy from your Clerk dashboard → API Keys, or set VITE_DISABLE_AUTH=true to run ' +
      'locally without sign-in (see .env.example).',
  );
}

// When auth is disabled for local dev, the sign-in flow is never invoked, so a
// placeholder publishable key is enough to satisfy ClerkProvider's constructor.
const FALLBACK_DEV_KEY = 'pk_test_bGFid2lyZWQtbG9jYWwtZGV2LmNsZXJrLmFjY291bnRzLmRldiQ';

export const CLERK_PUBLISHABLE_KEY = key ?? FALLBACK_DEV_KEY;
