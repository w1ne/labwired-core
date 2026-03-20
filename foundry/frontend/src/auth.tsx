import { createContext, useContext } from 'react';
import type { ReactNode } from 'react';
import {
  ClerkProvider,
  UserButton as ClerkUserButton,
  useAuth as useClerkAuth,
  useClerk as useClerkClient,
} from '@clerk/react';

type AuthContextValue = {
  clerkEnabled: boolean;
  isLoaded: boolean;
  isSignedIn: boolean;
  getToken: () => Promise<string | null>;
  openSignIn: (options?: { fallbackRedirectUrl?: string }) => void;
};

const clerkPublishableKey = import.meta.env.VITE_CLERK_PUBLISHABLE_KEY as string | undefined;
const clerkEnabled = Boolean(clerkPublishableKey);

const fallbackAuthContext: AuthContextValue = {
  clerkEnabled: false,
  isLoaded: true,
  isSignedIn: false,
  getToken: async () => null,
  openSignIn: () => {},
};

const AuthContext = createContext<AuthContextValue>(fallbackAuthContext);

function ClerkBackedAuthProvider({ children }: { children: ReactNode }) {
  const auth = useClerkAuth();
  const clerk = useClerkClient();

  return (
    <AuthContext.Provider
      value={{
        clerkEnabled: true,
        isLoaded: auth.isLoaded,
        isSignedIn: Boolean(auth.isSignedIn),
        getToken: auth.getToken,
        openSignIn: clerk.openSignIn,
      }}
    >
      {children}
    </AuthContext.Provider>
  );
}

export function AuthProvider({ children }: { children: ReactNode }) {
  if (!clerkEnabled) {
    return <AuthContext.Provider value={fallbackAuthContext}>{children}</AuthContext.Provider>;
  }

  return (
    <ClerkProvider publishableKey={clerkPublishableKey!} afterSignOutUrl="/">
      <ClerkBackedAuthProvider>{children}</ClerkBackedAuthProvider>
    </ClerkProvider>
  );
}

export function useAuth() {
  return useContext(AuthContext);
}

export function useClerk() {
  const { openSignIn, clerkEnabled } = useContext(AuthContext);
  return { openSignIn, clerkEnabled };
}

export function UserButton(props: Record<string, unknown>) {
  if (!clerkEnabled) {
    return null;
  }
  return <ClerkUserButton {...props} />;
}
