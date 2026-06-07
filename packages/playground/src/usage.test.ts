import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { trackUsage } from './usage';

describe('trackUsage (playground beacon)', () => {
  const fetchMock = vi.fn(() => Promise.resolve(new Response(null, { status: 204 })));

  beforeEach(() => {
    vi.stubGlobal('fetch', fetchMock);
    fetchMock.mockClear();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('POSTs the event and board to /v1/events', () => {
    vi.stubGlobal('navigator', { doNotTrack: '0' });
    trackUsage('run_clicked', { board: 'stm32f103' });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toMatch(/\/v1\/events$/);
    expect(JSON.parse(init.body as string)).toEqual({ event: 'run_clicked', board: 'stm32f103' });
    expect(init.keepalive).toBe(true);
  });

  it('sends nothing when Do Not Track is on', () => {
    vi.stubGlobal('navigator', { doNotTrack: '1' });
    trackUsage('app_loaded');
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('sends nothing when Global Privacy Control is on', () => {
    vi.stubGlobal('navigator', { doNotTrack: '0', globalPrivacyControl: true });
    trackUsage('app_loaded');
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('never throws when fetch rejects', () => {
    vi.stubGlobal('navigator', { doNotTrack: '0' });
    fetchMock.mockImplementationOnce(() => Promise.reject(new Error('offline')));
    expect(() => trackUsage('board_selected', { board: 'rp2040' })).not.toThrow();
  });
});
