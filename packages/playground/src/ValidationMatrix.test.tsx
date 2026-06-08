import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { ValidationMatrix, MATRIX_URL } from './ValidationMatrix';

const SAMPLE = {
  esp32s3: {
    gpio: { status: 'pass', run_url: 'https://github.com/w1ne/labwired-core/actions/runs/1' },
    spi: { status: 'pass' }, // universal, no evidence -> must render unrecorded
    mcpwm: { status: 'pass', run_url: 'https://github.com/w1ne/labwired-core/actions/runs/1' }, // chip-specific -> excluded from overview
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
    await waitFor(() => expect(screen.getByText('ESP32-S3 (Xtensa LX7)')).toBeTruthy());
    expect(fetch).toHaveBeenCalledWith(MATRIX_URL, expect.objectContaining({ signal: expect.any(AbortSignal) }));
    const gpioLink = screen.getByRole('link', { name: /gpio: pass/i });
    expect(gpioLink.getAttribute('href')).toContain('/actions/runs/1');
  });

  it('downgrades evidence-less cells to unrecorded (proof-artifact bar)', async () => {
    render(<ValidationMatrix />);
    await waitFor(() => expect(screen.getByText('ESP32-S3 (Xtensa LX7)')).toBeTruthy());
    expect(screen.getByLabelText('spi: unrecorded')).toBeTruthy();
    expect(screen.queryByRole('link', { name: /spi: pass/i })).toBeNull();
  });

  it('shows a graceful empty state when the fetch fails', async () => {
    (fetch as ReturnType<typeof vi.fn>).mockImplementationOnce(() => Promise.reject(new Error('offline')));
    render(<ValidationMatrix />);
    await waitFor(() => expect(screen.getByText(/validation data unavailable/i)).toBeTruthy());
  });

  it('renders na status as unlinked "not modeled" 🚧 with correct aria-label', async () => {
    render(<ValidationMatrix />);
    await waitFor(() => expect(screen.getByText('ESP32-S3 (Xtensa LX7)')).toBeTruthy());
    const naCell = screen.getByLabelText('uart: na');
    expect(naCell.tagName.toLowerCase()).not.toBe('a');
    expect(naCell.textContent).toBe('🚧');
  });

  it('overview is the 12 universal subsystems only; chip-specific classes are excluded', async () => {
    render(<ValidationMatrix />);
    await waitFor(() => expect(screen.getByText('ESP32-S3 (Xtensa LX7)')).toBeTruthy());
    const headers = screen.getAllByRole('columnheader').map((th) => th.textContent?.toLowerCase());
    const clockIdx = headers.findIndex((h) => h === 'clock');
    const irqIdx = headers.findIndex((h) => h === 'irq');
    expect(clockIdx).toBeGreaterThanOrEqual(0);
    expect(irqIdx).toBeGreaterThan(clockIdx);
    // The 12 universal subsystems are present; chip-specific peripherals
    // (e.g. ESP32 RMT/MCPWM) are intentionally not columns in the overview.
    for (const cls of ['clock', 'gpio', 'uart', 'timer', 'dma', 'irq', 'i2c', 'spi', 'adc', 'pwm', 'wdt', 'rtc']) {
      expect(headers).toContain(cls);
    }
    expect(headers).not.toContain('mcpwm');
    expect(headers).not.toContain('rmt');
  });
});
