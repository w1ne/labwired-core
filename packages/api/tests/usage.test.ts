import { describe, it, expect, vi } from 'vitest';
import { trackUsage, handleTrackEvent, WEB_EVENT_ALLOWLIST } from '../src/usage.js';
import { callHostedTool } from '../src/mcp/tools.js';

function mockUsage() {
  return { writeDataPoint: vi.fn() };
}

describe('trackUsage', () => {
  it('is a no-op when the USAGE binding is absent', () => {
    // Must not throw — local dev and tests run without Analytics Engine.
    expect(() => trackUsage({} as any, { event: 'mcp_tool', tool: 'labwired_run' })).not.toThrow();
  });

  it('writes one data point with event/tool/board/status blobs', () => {
    const usage = mockUsage();
    trackUsage({ USAGE: usage } as any, {
      event: 'mcp_tool',
      tool: 'labwired_run',
      board: 'stm32l476',
      status: 'ok',
      durationMs: 123,
    });
    expect(usage.writeDataPoint).toHaveBeenCalledTimes(1);
    const point = usage.writeDataPoint.mock.calls[0][0];
    expect(point.blobs).toEqual(['mcp_tool', 'labwired_run', 'stm32l476', 'ok', 'api']);
    expect(point.doubles).toEqual([123]);
    expect(point.indexes).toEqual(['mcp_tool']);
  });

  it('never throws even when the binding write fails', () => {
    const usage = { writeDataPoint: vi.fn(() => { throw new Error('boom'); }) };
    expect(() => trackUsage({ USAGE: usage } as any, { event: 'mcp_tool' })).not.toThrow();
  });
});

describe('POST /v1/events (web beacons)', () => {
  function req(body: unknown): Request {
    return new Request('https://api.labwired.com/v1/events', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    });
  }

  it('accepts an allowlisted event and writes a data point', async () => {
    const usage = mockUsage();
    const res = await handleTrackEvent(req({ event: 'run_clicked', board: 'stm32f103' }), {
      USAGE: usage,
    } as any);
    expect(res.status).toBe(204);
    const point = usage.writeDataPoint.mock.calls[0][0];
    expect(point.blobs[0]).toBe('run_clicked');
    expect(point.blobs[2]).toBe('stm32f103');
    expect(point.blobs[4]).toBe('web');
  });

  it('rejects events not on the allowlist', async () => {
    const usage = mockUsage();
    const res = await handleTrackEvent(req({ event: 'drop_table_users' }), { USAGE: usage } as any);
    expect(res.status).toBe(400);
    expect(usage.writeDataPoint).not.toHaveBeenCalled();
  });

  it('rejects malformed JSON without throwing', async () => {
    const usage = mockUsage();
    const raw = new Request('https://api.labwired.com/v1/events', {
      method: 'POST',
      body: 'not json',
    });
    const res = await handleTrackEvent(raw, { USAGE: usage } as any);
    expect(res.status).toBe(400);
  });

  it('truncates oversized field values instead of storing them', async () => {
    const usage = mockUsage();
    const res = await handleTrackEvent(req({ event: 'lab_opened', board: 'x'.repeat(500) }), {
      USAGE: usage,
    } as any);
    expect(res.status).toBe(204);
    const point = usage.writeDataPoint.mock.calls[0][0];
    expect((point.blobs[2] as string).length).toBeLessThanOrEqual(64);
  });

  it('still returns 204 when the USAGE binding is absent (beacon never errors)', async () => {
    const res = await handleTrackEvent(req({ event: 'app_loaded' }), {} as any);
    expect(res.status).toBe(204);
  });

  it('allowlist covers the playground instrumentation events', () => {
    for (const e of ['app_loaded', 'board_selected', 'run_clicked', 'lab_opened']) {
      expect(WEB_EVENT_ALLOWLIST.has(e)).toBe(true);
    }
  });
});

describe('MCP tool-call usage instrumentation', () => {
  it('records one mcp_tool event per callHostedTool dispatch', async () => {
    const usage = mockUsage();
    const env = {
      BUILDER_URL: 'https://b',
      BUILDER_SECRET: 'k',
      ENVIRONMENT: 'test',
      USAGE: usage,
    } as any;
    await callHostedTool({ name: 'labwired_list_components', arguments: {} }, env, { userId: 'u' });
    expect(usage.writeDataPoint).toHaveBeenCalledTimes(1);
    const point = usage.writeDataPoint.mock.calls[0][0];
    expect(point.blobs[0]).toBe('mcp_tool');
    expect(point.blobs[1]).toBe('labwired_list_components');
    expect(point.blobs[3]).toBe('ok');
  });

  it('records board and error status for a failed labwired_run', async () => {
    const usage = mockUsage();
    const env = {
      BUILDER_URL: 'https://b',
      BUILDER_SECRET: 'k',
      ENVIRONMENT: 'test',
      USAGE: usage,
    } as any;
    await callHostedTool(
      {
        name: 'labwired_run',
        arguments: {
          elf_base64: 'AA==',
          target: 'stm32l476',
          diagram: { board: 'rp2040', parts: [], wires: [] },
        },
      },
      env,
      { userId: 'u' },
    );
    const point = usage.writeDataPoint.mock.calls[0][0];
    expect(point.blobs[1]).toBe('labwired_run');
    expect(point.blobs[2]).toBe('stm32l476');
    expect(point.blobs[3]).toBe('error');
  });
});
