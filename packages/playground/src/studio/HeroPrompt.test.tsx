import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { HeroPrompt } from './HeroPrompt';

describe('HeroPrompt', () => {
  it('renders the hero prompt placeholder', () => {
    render(<HeroPrompt onFocus={() => {}} />);
    expect(screen.getByPlaceholderText(/describe what to build/i)).toBeInTheDocument();
  });

  it('invokes onFocus when the input is focused', async () => {
    const onFocus = vi.fn();
    render(<HeroPrompt onFocus={onFocus} />);
    await userEvent.click(screen.getByPlaceholderText(/describe what to build/i));
    expect(onFocus).toHaveBeenCalled();
  });
});
