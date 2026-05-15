import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
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
    render(<AuthPill auth={makeAuth()} onOpen={() => {}} />);
    expect(screen.getByRole('button', { name: /connect/i })).toBeInTheDocument();
  });

  it('renders plan + quota percent when signed in', () => {
    render(
      <AuthPill
        auth={makeAuth({ status: 'ok', workspace: proWorkspace })}
        onOpen={() => {}}
      />,
    );
    // 47M / 100M = 47%
    expect(screen.getByText(/Pro · 47%/)).toBeInTheDocument();
  });

  it('calls onOpen when clicked', async () => {
    const onOpen = vi.fn();
    render(<AuthPill auth={makeAuth()} onOpen={onOpen} />);
    await userEvent.click(screen.getByRole('button', { name: /connect/i }));
    expect(onOpen).toHaveBeenCalledOnce();
  });
});
