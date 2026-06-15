import type { McpTool } from './types.js';

const HOSTED_SECURITY_SCHEMES = [
  { type: 'oauth2', scopes: [] },
  { type: 'http', scheme: 'bearer' },
] as const;

const READ_ONLY_TOOLS = new Set([
  'labwired_search_tools',
  'labwired_list_boards',
  'labwired_list_components',
  'labwired_validate_diagram',
]);

const TITLES: Record<string, string> = {
  labwired_start_playground_lab: 'Start Playground Lab',
  labwired_list_boards: 'List Boards',
  labwired_validate_diagram: 'Validate Diagram',
  labwired_run: 'Run Firmware',
  labwired_list_components: 'List Components',
  labwired_search_tools: 'Search Tools',
  labwired_compile_diagram: 'Compile Diagram',
  labwired_open_hardware_lab: 'Open Hardware Lab',
};

export function toolTitle(name: string): string {
  return TITLES[name] ?? name
    .replace(/^labwired_/, '')
    .split('_')
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

export function toolAnnotations(name: string): NonNullable<McpTool['annotations']> {
  const title = toolTitle(name);
  const readOnly = READ_ONLY_TOOLS.has(name);
  return {
    title,
    readOnlyHint: readOnly,
    destructiveHint: false,
    ...(readOnly ? {} : { openWorldHint: true }),
  };
}

export function decorateTool(tool: McpTool): McpTool {
  const title = toolTitle(tool.name);
  return {
    ...tool,
    title,
    securitySchemes: [...HOSTED_SECURITY_SCHEMES],
    _meta: {
      ...(tool._meta ?? {}),
      securitySchemes: [...HOSTED_SECURITY_SCHEMES],
      'openai/toolInvocation/invoking': `${title} running`,
      'openai/toolInvocation/invoked': `${title} finished`,
    },
    annotations: toolAnnotations(tool.name),
  };
}

export function decorateTools(tools: readonly McpTool[]): McpTool[] {
  return tools.map(decorateTool);
}
