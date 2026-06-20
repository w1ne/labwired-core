import { describe, it, expect } from 'vitest';
import { listHostedTools, callHostedTool } from '../src/mcp/tools.js';

function makeKvStub() {
  const store = new Map<string, string>();
  return {
    get: (key: string) => Promise.resolve(store.get(key) ?? null),
    put: (key: string, value: string) => {
      store.set(key, value);
      return Promise.resolve();
    },
    _store: store,
  };
}

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
      // OpenAI requires all three hints set to an explicit boolean on every tool.
      expect(tool.annotations?.readOnlyHint, tool.name).toBeTypeOf('boolean');
      expect(tool.annotations?.openWorldHint, tool.name).toBeTypeOf('boolean');
      expect(tool.annotations?.destructiveHint, tool.name).toBeTypeOf('boolean');
    }
    const run = listHostedTools().find((t) => t.name === 'labwired_run');
    expect(run?.annotations).toMatchObject({ readOnlyHint: false, openWorldHint: true });
    const listBoards = listHostedTools().find((t) => t.name === 'labwired_list_boards');
    expect(listBoards?.annotations).toMatchObject({ readOnlyHint: true, openWorldHint: false });
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

  it('labwired_list_boards returns real Playground catalog ids, not invented aliases', async () => {
    const res = await callHostedTool({
      name: 'labwired_list_boards',
      arguments: {},
    }, { ENVIRONMENT: 'test' } as any, { userId: 'user_abc' });

    const text = JSON.parse(res.content[0].text);
    const ids = text.boards.map((board: { id: string }) => board.id);
    expect(ids).toContain('stm32f103-blinky');
    expect(ids).toContain('nucleo-f401re');
    expect(ids).not.toContain('stm32l476-blinky');
  });

  it('labwired_open_hardware_lab advertises an embedded ChatGPT component', () => {
    const tool = listHostedTools().find((t) => t.name === 'labwired_open_hardware_lab');
    expect(tool).toBeDefined();
    expect(tool!.annotations).toMatchObject({ readOnlyHint: false, destructiveHint: false, openWorldHint: true });
    expect(tool!._meta).toMatchObject({
      'openai/outputTemplate': 'ui://widget/labwired-hardware-lab-v8.html',
      'openai/widgetAccessible': true,
      ui: {
        resourceUri: 'ui://widget/labwired-hardware-lab-v8.html',
      },
      widgetAccessible: true,
      invoking: expect.any(String),
      invoked: expect.any(String),
    });
  });

  it('labwired_start_playground_lab advertises the same embedded component', () => {
    const tool = listHostedTools().find((t) => t.name === 'labwired_start_playground_lab');
    expect(tool).toBeDefined();
    expect(tool!._meta).toMatchObject({
      'openai/outputTemplate': 'ui://widget/labwired-hardware-lab-v8.html',
      'openai/widgetAccessible': true,
      ui: {
        resourceUri: 'ui://widget/labwired-hardware-lab-v8.html',
      },
    });
  });

  it('labwired_open_hardware_lab returns current Apps SDK component metadata', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool({
      name: 'labwired_open_hardware_lab',
      arguments: {},
    }, env, { userId: 'user_abc' });

    expect(res._meta).toMatchObject({
      'openai/outputTemplate': 'ui://widget/labwired-hardware-lab-v8.html',
      ui: {
        resourceUri: 'ui://widget/labwired-hardware-lab-v8.html',
      },
      'openai/widgetAccessible': true,
      widgetAccessible: true,
    });
  });

  it('labwired_start_playground_lab returns Studio links and an inline component without legacy watch_url', async () => {
    const kvProjects = makeKvStub();
    const env = {
      ENVIRONMENT: 'test',
      KV_PROJECTS: kvProjects,
    } as any;
    const res = await callHostedTool({
      name: 'labwired_start_playground_lab',
      arguments: {},
    }, env, { userId: 'user_abc' });

    expect(res.isError).toBeFalsy();
    expect(res.structuredContent).toMatchObject({
      ok: true,
      inline_component_uri: 'ui://widget/labwired-hardware-lab-v8.html',
      inline_frame_url: expect.stringContaining('https://app.labwired.com/?embed=true&share='),
      studio_url: expect.stringContaining('https://app.labwired.com/?share='),
      share_url: expect.stringContaining('https://app.labwired.com/?share='),
      scene: {
        board: 'stm32f103',
      },
    });
    expect(res._meta).toMatchObject({
      'openai/outputTemplate': 'ui://widget/labwired-hardware-lab-v8.html',
    });
    const text = JSON.parse(res.content[0].text);
    expect(text).toMatchObject({
      studio_url: expect.stringContaining('https://app.labwired.com/?share='),
      share_url: expect.stringContaining('https://app.labwired.com/?share='),
      inline_component_uri: 'ui://widget/labwired-hardware-lab-v8.html',
      inline_frame_url: expect.stringContaining('https://app.labwired.com/?embed=true&share='),
    });
    expect(text.studio_url.length).toBeLessThan(90);
    expect(text.studio_url).not.toContain('?watch=');
    expect(text.studio_url).not.toContain('?lab=');
    expect(text.studio_url).not.toContain('#');
    expect(text).not.toHaveProperty('watch_url');

    const shareId = new URL(text.studio_url).searchParams.get('share');
    expect(shareId).toBeTruthy();
    const payload = JSON.parse(kvProjects._store.get(`share:${shareId}`)!);
    expect(payload.source).toContain('int main');
    expect(payload.diagram).toMatchObject({
      version: 1,
      board: 'stm32f103',
      parts: [
        { id: 'mcu', attrs: {} },
        { id: 'led1', attrs: { color: 'green' } },
      ],
      wires: [
        {
          from: { part: 'mcu', pin: 'PA5' },
          to: { part: 'led1', pin: 'A' },
          color: '#3DD68C',
        },
      ],
    });
    expect(payload.diagram.parts[1]).not.toHaveProperty('color');
  });

  it('labwired_open_hardware_lab returns one ?share= link carrying the diagram', async () => {
    const kvProjects = makeKvStub();
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test', KV_PROJECTS: kvProjects } as any;
    const res = await callHostedTool({
      name: 'labwired_open_hardware_lab',
      arguments: {
        diagram: { board: 'nrf52840', parts: [{ id: 'mcu', type: 'nrf52840-dk' }, { id: 'd', type: 'ultrasonic' }, { id: 'l', type: 'led' }], wires: [] },
      },
    }, env, { userId: 'user_abc' });

    expect(res.isError).toBeFalsy();
    // One shareable link format for everything; the Playground runs it (the
    // share's own binary, or a matched example's firmware).
    expect(res.structuredContent.studio_url).toContain('?share=');
    expect(res.structuredContent.inline_frame_url).toContain('?embed=true&run=1&share=');
    const shareId = new URL(res.structuredContent.studio_url).searchParams.get('share');
    const payload = JSON.parse(kvProjects._store.get(`share:${shareId}`)!);
    expect(payload.diagram.board).toBe('nrf52840');
  });

  it('labwired_open_hardware_lab rejects boards that are not in the Playground catalog contract', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool({
      name: 'labwired_open_hardware_lab',
      arguments: {
        diagram: { board: 'stm32l999', parts: [{ id: 'mcu', type: 'mcu' }], wires: [] },
      },
    }, env, { userId: 'user_abc' });

    expect(res.isError).toBe(true);
    const text = JSON.parse(res.content[0].text);
    expect(text.error).toBe('BOARD_NOT_IN_PLAYGROUND_CATALOG');
    expect(text.detail).toContain('labwired_list_boards');
  });

  it('labwired_open_hardware_lab returns Studio links, scene shell, and component template hint', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test', KV_PROJECTS: makeKvStub() } as any;
    const res = await callHostedTool({
      name: 'labwired_open_hardware_lab',
      arguments: {
        diagram: {
          board: 'stm32f103',
          parts: [{ id: 'mcu', type: 'stm32f103' }],
          wires: [],
        },
      },
    }, env, { userId: 'user_abc' });
    expect(res.isError).toBeFalsy();
    expect(res.structuredContent).toMatchObject({
      ok: true,
      inline_component_uri: 'ui://widget/labwired-hardware-lab-v8.html',
      inline_frame_url: expect.stringContaining('https://app.labwired.com/?embed=true&run=1&share='),
      studio_url: expect.stringContaining('https://app.labwired.com/'),
      share_url: expect.stringContaining('https://app.labwired.com/'),
      scene: {
        board: 'stm32f103',
        parts: [{ id: 'mcu', type: 'stm32f103' }],
        wires: [],
      },
    });
    expect(res._meta).toMatchObject({
      'openai/outputTemplate': 'ui://widget/labwired-hardware-lab-v8.html',
    });
    const text = JSON.parse(res.content[0].text);
    expect(text).toMatchObject({
      inline_component_uri: 'ui://widget/labwired-hardware-lab-v8.html',
      inline_frame_url: expect.stringContaining('https://app.labwired.com/?embed=true&run=1&share='),
      studio_url: expect.stringContaining('https://app.labwired.com/'),
      share_url: expect.stringContaining('https://app.labwired.com/'),
    });
    expect(res.structuredContent.inline_frame_url).not.toContain('?watch=');
    expect(res.structuredContent.studio_url).not.toContain('?watch=');
    expect(res.structuredContent.share_url).not.toContain('?watch=');
    expect(res.structuredContent).not.toHaveProperty('watch_url');
    expect(res.structuredContent).not.toHaveProperty('template_uri');
    expect(text).not.toHaveProperty('watch_url');
    expect(text).not.toHaveProperty('template_uri');
  });

  it('labwired_open_hardware_lab normalizes agent diagrams into the shared record', async () => {
    const kvProjects = makeKvStub();
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test', KV_PROJECTS: kvProjects } as any;
    const res = await callHostedTool({
      name: 'labwired_open_hardware_lab',
      arguments: {
        diagram: {
          board: 'stm32f103',
          parts: [
            { id: 'mcu', type: 'mcu', label: 'STM32F103' },
            { id: 'led1', type: 'led', label: 'LED', color: 'green' },
          ],
          wires: [
            { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'A' } },
          ],
        },
      },
    }, env, { userId: 'user_abc' });

    expect(res.isError).toBeFalsy();
    expect(res.structuredContent.studio_url).toContain('?share=');
    const shareId = new URL(res.structuredContent.studio_url).searchParams.get('share');
    const payload = JSON.parse(kvProjects._store.get(`share:${shareId}`)!);
    expect(payload.diagram).toMatchObject({
      version: 1,
      parts: [
        { id: 'mcu', attrs: {} },
        { id: 'led1', attrs: { color: 'green' } },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'A' }, color: '#e83e8c' },
      ],
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

  it('labwired_compile_diagram rejects a diagram with a hallucinated peripheral pin', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool({
      name: 'labwired_compile_diagram',
      arguments: {
        diagram: {
          board: 'esp32-s3-zero',
          parts: [
            { id: 'mcu', type: 'esp32-s3-zero' },
            { id: 'led1', type: 'rgb-led' },
          ],
          // 'DIN' does not exist on rgb-led (R/G/B/GND) — must not compile.
          wires: [{ from: { part: 'mcu', pin: 'GPIO8' }, to: { part: 'led1', pin: 'DIN' } }],
        },
      },
    }, env, { userId: 'u' });
    expect(res.isError).toBe(true);
    const body = JSON.parse(res.content[0].text);
    expect(body.error).toBe('DIAGRAM_INVALID');
    expect(body.validation.ok).toBe(false);
  });

  it('labwired_run rejects an invalid diagram before reaching the simulator', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool({
      name: 'labwired_run',
      arguments: {
        elf_base64: 'AA==',
        target: 'esp32-s3-zero',
        diagram: {
          board: 'esp32-s3-zero',
          parts: [
            { id: 'mcu', type: 'esp32-s3-zero' },
            { id: 'led1', type: 'rgb-led' },
          ],
          wires: [{ from: { part: 'mcu', pin: 'GPIO8' }, to: { part: 'led1', pin: 'DIN' } }],
        },
        max_steps: 1000,
      },
    }, env, { userId: 'u' });
    expect(res.isError).toBe(true);
    expect(JSON.parse(res.content[0].text).error).toBe('DIAGRAM_INVALID');
  });

  it('advertises labwired_compile_firmware and labwired_build_and_run', () => {
    const names = listHostedTools().map((t) => t.name);
    expect(names).toContain('labwired_compile_firmware');
    expect(names).toContain('labwired_build_and_run');
  });

  it('labwired_compile_firmware rejects an unsupported board before calling the builder', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool(
      { name: 'labwired_compile_firmware', arguments: { source: 'int main(){return 0;}', board: 'commodore64' } },
      env, { userId: 'u' },
    );
    expect(res.isError).toBe(true);
    expect(JSON.parse(res.content[0].text).error).toBe('BOARD_NOT_COMPILABLE');
  });

  it('labwired_compile_firmware rejects empty source', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool(
      { name: 'labwired_compile_firmware', arguments: { source: '   ', board: 'stm32l476' } },
      env, { userId: 'u' },
    );
    expect(res.isError).toBe(true);
    expect(JSON.parse(res.content[0].text).error).toBe('INVALID_ARGS');
  });

  it('labwired_build_and_run refuses a compile-only board (ESP32) before building', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool(
      { name: 'labwired_build_and_run', arguments: { source: 'int main(){return 0;}', board: 'esp32-s3-zero', diagram: { board: 'esp32-s3-zero', parts: [], wires: [] } } },
      env, { userId: 'u' },
    );
    expect(res.isError).toBe(true);
    expect(JSON.parse(res.content[0].text).error).toBe('BOARD_NOT_RUNNABLE');
  });

  it('labwired_build_and_run rejects an invalid diagram before building', async () => {
    const env = { BUILDER_URL: 'https://b', BUILDER_SECRET: 'k', ENVIRONMENT: 'test' } as any;
    const res = await callHostedTool(
      { name: 'labwired_build_and_run', arguments: {
        source: 'int main(){return 0;}', board: 'stm32l476',
        diagram: { board: 'stm32l476', parts: [{ id: 'mcu', type: 'mcu' }, { id: 'led1', type: 'rgb-led' }], wires: [{ from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'DIN' } }] },
      } },
      env, { userId: 'u' },
    );
    expect(res.isError).toBe(true);
    expect(JSON.parse(res.content[0].text).error).toBe('DIAGRAM_INVALID');
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
