import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';

const mockClerkUser: { isLoaded: boolean; isSignedIn: boolean } = {
  isLoaded: true,
  isSignedIn: false,
};

vi.mock('@clerk/clerk-react', () => ({
  useUser: () => mockClerkUser,
  UserButton: () => <div data-testid="clerk-user-button">UserButton</div>,
  SignInButton: ({ children }: { children: ReactNode }) => (
    <div data-testid="clerk-sign-in-button">{children}</div>
  ),
}));

import { AuthPill } from './AuthPill';

describe('AuthPill', () => {
  it('renders a placeholder while Clerk is still loading', () => {
    mockClerkUser.isLoaded = false;
    mockClerkUser.isSignedIn = false;
    render(<AuthPill />);
    expect(screen.getByLabelText(/Loading account/i)).toBeInTheDocument();
  });

  it('wraps a "Sign in" button in Clerk SignInButton when signed out', () => {
    mockClerkUser.isLoaded = true;
    mockClerkUser.isSignedIn = false;
    render(<AuthPill />);
    expect(screen.getByTestId('clerk-sign-in-button')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Sign in/i })).toBeInTheDocument();
  });

  it('renders Clerk UserButton when signed in', () => {
    mockClerkUser.isLoaded = true;
    mockClerkUser.isSignedIn = true;
    render(<AuthPill />);
    expect(screen.getByTestId('clerk-user-button')).toBeInTheDocument();
  });
});
