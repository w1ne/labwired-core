import { useUser, UserButton, SignInButton } from '@clerk/clerk-react';

// Single Clerk-only auth surface. Signed-out users click "Sign in", which opens
// Clerk's hosted modal directly (no intermediate wrapper). Signed-in users see
// Clerk's UserButton with profile/sign-out built in.
export function AuthPill() {
  const { isLoaded, isSignedIn } = useUser();

  if (!isLoaded) {
    return (
      <span
        aria-label="Loading account"
        className="h-7 px-3 rounded-pill text-xs font-medium bg-white/[0.05] text-fg-tertiary flex items-center shrink-0"
      >
        …
      </span>
    );
  }

  if (isSignedIn) {
    return (
      <div className="flex items-center shrink-0">
        <UserButton
          afterSignOutUrl="/"
          appearance={{ elements: { avatarBox: 'w-7 h-7' } }}
        />
      </div>
    );
  }

  return (
    <SignInButton mode="modal">
      <button
        type="button"
        aria-label="Sign in to LabWired"
        className="h-7 px-3 rounded-pill text-xs font-medium bg-accent text-bg-base hover:bg-accent/90 transition-colors duration-micro border-0 outline-none focus-visible:ring-2 focus-visible:ring-accent/50 flex items-center gap-1.5 shrink-0"
      >
        Sign in
      </button>
    </SignInButton>
  );
}
