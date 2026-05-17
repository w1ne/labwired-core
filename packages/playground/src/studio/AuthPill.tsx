import { useUser, UserButton, SignInButton } from '@clerk/clerk-react';

export interface AuthPillProps {
  onOpenProjects?: () => void;
}

// Single Clerk-only auth surface. Signed-out users click "Sign in", which opens
// Clerk's hosted modal directly (no intermediate wrapper). Signed-in users see
// Clerk's UserButton with profile/sign-out built in. When onOpenProjects is
// passed, a "My projects" entry is injected into the avatar dropdown.
export function AuthPill({ onOpenProjects }: AuthPillProps = {}) {
  const { isLoaded, isSignedIn } = useUser();

  if (!isLoaded) {
    // Avatar-shaped skeleton so the toolbar doesn't shift width once Clerk
    // resolves. Matches the w-7 h-7 of the actual UserButton avatar below.
    return (
      <span
        aria-label="Loading account"
        className="w-7 h-7 rounded-full bg-white/[0.05] shrink-0 animate-pulse"
      />
    );
  }

  if (isSignedIn) {
    return (
      <div className="flex items-center shrink-0">
        <UserButton
          afterSignOutUrl="/"
          appearance={{ elements: { avatarBox: 'w-7 h-7' } }}
        >
          {onOpenProjects && (
            <UserButton.MenuItems>
              <UserButton.Action
                label="My projects"
                labelIcon={<FolderIcon />}
                onClick={onOpenProjects}
              />
            </UserButton.MenuItems>
          )}
        </UserButton>
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

function FolderIcon() {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M2 4a1 1 0 0 1 1-1h3l2 2h5a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1z" />
    </svg>
  );
}
