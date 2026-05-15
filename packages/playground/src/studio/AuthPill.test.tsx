import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

// Mock Clerk before importing AuthPill — controlled per-test via mockClerkUser.
const mockClerkUser: {
  isLoaded: boolean;
  isSignedIn: boolean;
  user: { id?: string; primaryEmailAddress?: { emailAddress: string } } | null;
} = {
  isLoaded: true,
  isSignedIn: false,
  user: null,
};

vi.mock('@clerk/clerk-react', () => ({
  useUser: () => mockClerkUser,
  UserButton: () => <div data-testid="clerk-user-button">UserButton</div>,
}));

import { AuthPill } from './AuthPill';
import type { UseAuthResult, Workspace } from './useAuth';

function makeAuth(overrides: Partial<UseAuthResult> = {}): UseAuthResult {
  return {
    apiKey: null,
    workspace: null,
    status: 'idle',
    error: null,
    save: vi.fn().mockResolvedValue(true),
    clear: vi.fn(),
    refresh: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

const proWorkspace: Workspace = {
  workspace_id: 'ws_test',
  plan: 'pro',
  status: 'active',
  cycles_used_mtd: 47_000_000,
  cycles_quota: 100_000_000,
  period_start_date: '2026-05-01T00:00:00Z',
  created_at: '2026-04-15T00:00:00Z',
};

describe('AuthPill', () => {
  it('renders "Connect" when signed out', () => {
    mockClerkUser.isLoaded = true;
    mockClerkUser.isSignedIn = false;
    mockClerkUser.user = null;
    render(<AuthPill auth={makeAuth()} onOpen={() => {}} />);
    expect(screen.getByRole('button', { name: /connect/i })).toBeInTheDocument();
  });

  it('renders plan + quota percent when signed in via API key', () => {
    mockClerkUser.isLoaded = true;
    mockClerkUser.isSignedIn = false;
    mockClerkUser.user = null;
    render(
      <AuthPill
        auth={makeAuth({ status: 'ok', workspace: proWorkspace })}
        onOpen={() => {}}
      />,
    );
    // 47M / 100M = 47%
    expect(screen.getByText(/Pro · 47%/)).toBeInTheDocument();
  });

  it('calls onOpen when "Connect" is clicked', async () => {
    mockClerkUser.isLoaded = true;
    mockClerkUser.isSignedIn = false;
    mockClerkUser.user = null;
    const onOpen = vi.fn();
    render(<AuthPill auth={makeAuth()} onOpen={onOpen} />);
    await userEvent.click(screen.getByRole('button', { name: /connect/i }));
    expect(onOpen).toHaveBeenCalledOnce();
  });

  it('renders Clerk UserButton when signed in via Clerk', () => {
    mockClerkUser.isLoaded = true;
    mockClerkUser.isSignedIn = true;
    mockClerkUser.user = {
      id: 'user_test',
      primaryEmailAddress: { emailAddress: 'andrii@example.com' },
    };
    render(<AuthPill auth={makeAuth()} onOpen={() => {}} />);
    expect(screen.getByTestId('clerk-user-button')).toBeInTheDocument();
  });
});
