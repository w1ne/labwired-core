import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import LandingPage from './LandingPage';

const openSignInMock = vi.fn();
const useAuthMock = vi.fn();

vi.mock('@clerk/react', () => ({
  useAuth: () => useAuthMock(),
  useClerk: () => ({ openSignIn: openSignInMock }),
  UserButton: () => <div data-testid="user-button" />,
}));

describe('LandingPage dashboard CTA', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal(
      'fetch',
      vi.fn().mockReturnValue(new Promise(() => {})),
    );
  });

  it('opens Clerk sign-in when signed out and key is configured', () => {
    vi.stubEnv('VITE_CLERK_PUBLISHABLE_KEY', 'pk_live_test');
    useAuthMock.mockReturnValue({ isSignedIn: false });

    render(<LandingPage onEnterDashboard={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /Dashboard/i }));

    expect(openSignInMock).toHaveBeenCalledWith({ fallbackRedirectUrl: '#/dashboard' });
  });

  it('enters dashboard directly when user is signed in', () => {
    vi.stubEnv('VITE_CLERK_PUBLISHABLE_KEY', 'pk_live_test');
    useAuthMock.mockReturnValue({ isSignedIn: true });
    const onEnterDashboard = vi.fn();

    render(<LandingPage onEnterDashboard={onEnterDashboard} />);
    fireEvent.click(screen.getByRole('button', { name: /Dashboard/i }));

    expect(onEnterDashboard).toHaveBeenCalledTimes(1);
    expect(openSignInMock).not.toHaveBeenCalled();
  });

  it('shows an alert when Clerk key is missing', () => {
    vi.stubEnv('VITE_CLERK_PUBLISHABLE_KEY', '');
    useAuthMock.mockReturnValue({ isSignedIn: false });
    const onEnterDashboard = vi.fn();
    const alertSpy = vi.spyOn(window, 'alert').mockImplementation(() => {});

    render(<LandingPage onEnterDashboard={onEnterDashboard} />);
    fireEvent.click(screen.getByRole('button', { name: /Dashboard/i }));

    expect(alertSpy).toHaveBeenCalledWith(
      'Dashboard login is not configured yet. Missing VITE_CLERK_PUBLISHABLE_KEY in frontend build.',
    );
    expect(onEnterDashboard).not.toHaveBeenCalled();
    expect(openSignInMock).not.toHaveBeenCalled();
  });
});
