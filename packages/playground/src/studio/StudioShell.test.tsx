import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { StudioShell } from './StudioShell';

describe('StudioShell', () => {
  it('renders the top chrome', () => {
    render(<StudioShell />);
    expect(screen.getByRole('banner')).toBeInTheDocument();
  });

  it('does NOT render an inspector by default', () => {
    render(<StudioShell />);
    expect(screen.queryByRole('complementary', { name: /inspector/i })).toBeNull();
  });

  it('renders the main canvas region', () => {
    render(<StudioShell />);
    expect(screen.getByRole('main', { name: /canvas/i })).toBeInTheDocument();
  });
});
