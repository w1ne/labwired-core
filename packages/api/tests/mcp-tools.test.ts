import { describe, it, expect } from 'vitest';
import { listHostedTools, callHostedTool } from '../src/mcp/tools.js';

describe('expanded MCP tools', () => {
  it('advertises run, list_components, list_boards, search, and compile_diagram', () => {
    const names = listHostedTools().map((t) => t.name);
    expect(names).toContain('labwired_run');
    expect(names).toContain('labwired_list_components');
    expect(names).toContain('labwired_list_boards');
    expect(names).toContain('labwired_search_tools');
    expect(names).toContain('labwired_compile_diagram');
    expect(names).toContain('labwired_open_hardware_lab');
    expect(names).not.toContain('labwired_compile');
  });

  it('adds MCP titles and risk annotations to every hosted tool', () => {
    for (const tool of listHostedTools()) {
      expect(tool.title, tool.name).toBeTypeOf('string');
      expect(tool.title, tool.name).not.toHaveLength(0);
      expect(tool.annotations, tool.name).toMatchObject({
        title: tool.title,
        destructiveHint: false,
      });
      expect(tool.annotations?.readOnlyHint, tool.name).toBeTypeOf('boolean');
    }
    const run = listHostedTools().find((t) => t.name === 'labwired_run');
    expect(run?.annotations).toMatchObject({ readOnlyHint: false, openWorldHint: true });
    const listBoards = listHostedTools().find((t) => t.name === 'labwired_list_boards');
    expect(listBoards?.annotations).toMatchObject({ readOnlyHint: true });
  });

  it('advertises ChatGPT-compatible security schemes on every hosted tool', () => {
    for (const tool of listHostedTools()) {
      expect(tool.securitySchemes, tool.name).toEqual([
        { type: 'oauth2', scopes: [] },
      ]);
      expect(tool.securitySchemes, tool.name).not.toContainEqual(expect.objectContaining({ type: 'http' }));
      expect(tool._meta, tool.name).toMatchObject({
        securitySchemes: tool.securitySchemes,
        'openai/toolInvocation/invoking': expect.any(String),
        'openai/toolInvocation/invoked': expect.any(String),
      });
    }
  });

  it('labwired_compile_diagram has readOnlyHint false and title "Compile Diagram"', () => {
    const compileTool = listHostedTools().find((t) => t.name === 'labwired_compile_diagram');
    expect(compileTool).toBeDefined();
    expect(compileTool!.title).toBe('Compile Diagram');
    expect(compileTool!.annotations?.readOnlyHint).toBe(false);
  });

  it('labwired_open_hardware_lab advertises an embedded ChatGPT component', () => {
    const tool = listHostedTools().find((t) => t.name === 'labwired_open_hardware_lab');
    expect(tool).toBeDefined();
    expect(tool!.annotations).toMatchObject({ readOnlyHint: false, destructiveHint: false, openWorldHint: true });
    expect(tool!._meta).toMatchObject({
      'openai/outputTemplate': 'ui://labwired/hardware-lab.html',
      'openai/widgetAccessible': true,
      ui: {
        resourceUri: 'ui://labwired/hardware-lab.html',
      },
      widgetAccessible: true,
      invoking: expect.any(String),
      invoked: expect.any(String),
    });
  });

  it('labwired_open_hardware_lab returns current Apps SDK component metadata', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool({
      name: 'labwired_open_hardware_lab',
      arguments: {},
    }, env, { userId: 'user_abc' });

    expect(res._meta).toMatchObject({
      'openai/outputTemplate': 'ui://labwired/hardware-lab.html',
      ui: {
        resourceUri: 'ui://labwired/hardware-lab.html',
      },
      'openai/widgetAccessible': true,
      widgetAccessible: true,
    });
  });

  it('labwired_open_hardware_lab returns a watch url, scene shell, and component template hint', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool({
      name: 'labwired_open_hardware_lab',
      arguments: {
        diagram: {
          board: 'stm32l476',
          parts: [{ id: 'mcu', type: 'stm32l476' }],
          wires: [],
        },
      },
    }, env, { userId: 'user_abc' });
    expect(res.isError).toBeFalsy();
    expect(res.structuredContent).toMatchObject({
      ok: true,
      inline_component_uri: 'ui://labwired/hardware-lab.html',
      studio_url: expect.stringContaining('https://app.labwired.com/'),
      share_url: expect.stringContaining('https://app.labwired.com/'),
      watch_url: expect.stringContaining('https://app.labwired.com/'),
      template_uri: 'ui://labwired/hardware-lab.html',
      scene: {
        board: 'stm32l476',
        parts: [{ id: 'mcu', type: 'stm32l476' }],
        wires: [],
      },
    });
    expect(res._meta).toMatchObject({
      'openai/outputTemplate': 'ui://labwired/hardware-lab.html',
    });
    const text = JSON.parse(res.content[0].text);
    expect(text).toMatchObject({
      inline_component_uri: 'ui://labwired/hardware-lab.html',
      studio_url: expect.stringContaining('https://app.labwired.com/'),
      share_url: expect.stringContaining('https://app.labwired.com/'),
    });
  });

  it('labwired_compile_diagram compiles a clean dispenser diagram', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool({
      name: 'labwired_compile_diagram',
      arguments: {
        diagram: {
          board: 'esp32-s3-zero',
          parts: [
            { id: 'mcu', type: 'esp32-s3-zero' },
            { id: 'pca1', type: 'pca9685', attrs: { i2c_address: '0x40' } },
          ],
          wires: [
            { from: { part: 'mcu', pin: 'GPIO8' }, to: { part: 'pca1', pin: 'SDA' } },
            { from: { part: 'mcu', pin: 'GPIO9' }, to: { part: 'pca1', pin: 'SCL' } },
            { from: { part: 'mcu', pin: '3V3' }, to: { part: 'pca1', pin: 'VCC' } },
            { from: { part: 'mcu', pin: 'GND' }, to: { part: 'pca1', pin: 'GND' } },
          ],
        },
      },
    }, env, { userId: 'u' });
    expect(res.isError).toBeFalsy();
    const body = JSON.parse(res.content[0].text);
    expect(body.ok).toBe(true);
    expect(body.system_yaml).toContain('i2c');
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

  it('labwired_search_tools finds diagram validation capability', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool(
      { name: 'labwired_search_tools', arguments: { query: 'diagram validation', limit: 3 } },
      env,
      { userId: 'u' },
    );
    const payload = JSON.parse(res.content[0].text);
    expect(payload.query).toBe('diagram validation');
    expect(payload.tools.map((tool: { name: string }) => tool.name)).toContain('labwired_validate_diagram');
    expect(payload.tools[0]).toHaveProperty('title');
    expect(payload.tools[0]).toHaveProperty('inputSchema');
    expect(payload.tools[0]).toHaveProperty('outputSchema');
  });

  it('labwired_search_tools returns guide and workflow hints for agents', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool(
      { name: 'labwired_search_tools', arguments: { query: 'build hardware run firmware inspect evidence', limit: 4 } },
      env,
      { userId: 'u' },
    );
    const payload = JSON.parse(res.content[0].text);
    expect(payload.guide_uri).toBe('labwired://guides/agent-hardware-loop');
    expect(payload.workflow).toEqual([
      'labwired_list_boards',
      'labwired_list_components',
      'labwired_validate_diagram',
      'labwired_compile_diagram',
      'labwired_run',
    ]);
    expect(payload.tools[0].annotations).toMatchObject({
      destructiveHint: false,
    });
  });
});
