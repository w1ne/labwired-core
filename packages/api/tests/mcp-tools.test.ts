import { describe, it, expect } from 'vitest';
import { listHostedTools, callHostedTool } from '../src/mcp/tools.js';

describe('expanded MCP tools', () => {
  it('advertises run, list_components, list_boards but NOT compile', () => {
    const names = listHostedTools().map((t) => t.name);
    expect(names).toContain('labwired_run');
    expect(names).toContain('labwired_list_components');
    expect(names).toContain('labwired_list_boards');
    expect(names).not.toContain('labwired_compile');
  });

  it('labwired_run rejects a target/board mismatch', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool({ name: 'labwired_run', arguments: { elf_base64: 'AA==', target: 'stm32l476', diagram: { board: 'rp2040', parts: [], wires: [] }, max_steps: 1000 } }, env, { userId: 'u' });
    expect(JSON.parse(res.content[0].text).error).toMatch(/mismatch/i);
  });

  it('labwired_run description mentions diagnosis and firmware-scaffolds', () => {
    const tool = listHostedTools().find((t) => t.name === 'labwired_run');
    expect(tool).toBeDefined();
    expect(tool!.description).toMatch(/diagnosis/i);
    expect(tool!.description).toMatch(/firmware-scaffolds/i);
  });

  it('labwired_list_components returns a non-empty list', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool({ name: 'labwired_list_components', arguments: {} }, env, { userId: 'u' });
    const payload = JSON.parse(res.content[0].text);
    expect(Array.isArray(payload.components)).toBe(true);
    expect(payload.components.length).toBeGreaterThan(0);
  });
});
