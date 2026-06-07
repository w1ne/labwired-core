import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { ValidationMatrix, MATRIX_URL } from './ValidationMatrix';

const SAMPLE = {
  esp32s3: {
    gpio: { status: 'pass', run_url: 'https://github.com/w1ne/labwired-core/actions/runs/1' },
    mcpwm: { status: 'pass' }, // no evidence -> must render unrecorded
    dma: { status: 'blocked', run_url: 'https://github.com/w1ne/labwired-core/actions/runs/1' },
    uart: { status: 'na' },
  },
};

describe('ValidationMatrix', () => {
  beforeEach(() => {
    vi.stubGlobal('fetch', vi.fn(() =>
      Promise.resolve(new Response(JSON.stringify(SAMPLE), { status: 200 })),
    ));
  });

  it('fetches the core-main snapshot and renders evidence-linked cells', async () => {
    render(<ValidationMatrix />);
    await waitFor(() => expect(screen.getByText('esp32s3')).toBeTruthy());
    expect(fetch).toHaveBeenCalledWith(MATRIX_URL, expect.objectContaining({ signal: expect.any(AbortSignal) }));
    const gpioLink = screen.getByRole('link', { name: /gpio: pass/i });
    expect(gpioLink.getAttribute('href')).toContain('/actions/runs/1');
  });

  it('downgrades evidence-less cells to unrecorded (proof-artifact bar)', async () => {
    render(<ValidationMatrix />);
    await waitFor(() => expect(screen.getByText('esp32s3')).toBeTruthy());
    expect(screen.getByLabelText('mcpwm: unrecorded')).toBeTruthy();
    expect(screen.queryByRole('link', { name: /mcpwm: pass/i })).toBeNull();
  });

  it('shows a graceful empty state when the fetch fails', async () => {
    (fetch as ReturnType<typeof vi.fn>).mockImplementationOnce(() => Promise.reject(new Error('offline')));
    render(<ValidationMatrix />);
    await waitFor(() => expect(screen.getByText(/validation data unavailable/i)).toBeTruthy());
  });

  it('renders na status as unlinked "—" with correct aria-label', async () => {
    render(<ValidationMatrix />);
    await waitFor(() => expect(screen.getByText('esp32s3')).toBeTruthy());
    const naCell = screen.getByLabelText('uart: na');
    expect(naCell.tagName.toLowerCase()).not.toBe('a');
    expect(naCell.textContent).toBe('—');
  });

  it('column order: rubric classes appear before extras', async () => {
    render(<ValidationMatrix />);
    await waitFor(() => expect(screen.getByText('esp32s3')).toBeTruthy());
    const headers = screen.getAllByRole('columnheader').map((th) => th.textContent?.toLowerCase());
    const clockIdx = headers.findIndex((h) => h === 'clock');
    const irqIdx = headers.findIndex((h) => h === 'irq');
    const mcpwmIdx = headers.findIndex((h) => h === 'mcpwm');
    expect(clockIdx).toBeGreaterThanOrEqual(0);
    expect(irqIdx).toBeGreaterThan(clockIdx);
    expect(mcpwmIdx).toBeGreaterThan(irqIdx);
  });
});
