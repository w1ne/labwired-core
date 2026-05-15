const key = import.meta.env.VITE_CLERK_PUBLISHABLE_KEY as string | undefined;

if (!key) {
  throw new Error(
    'VITE_CLERK_PUBLISHABLE_KEY is missing. Set it in packages/playground/.env.local — copy from your Clerk dashboard → API Keys.',
  );
}

export const CLERK_PUBLISHABLE_KEY = key;
