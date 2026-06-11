interface ToolLike {
  name: string;
  description: string;
  inputSchema: {
    type: string;
    properties?: Record<string, unknown>;
    required?: string[];
  };
}

const READ_ONLY_TOOLS = new Set([
  'labwired_search_tools',
  'labwired_catalog',
  'labwired_validate_system',
  'labwired_list_boards',
  'labwired_inspect_run',
  'labwired_validate_diagram',
]);

// labwired_define_component is intentionally absent from READ_ONLY_TOOLS:
// it writes a spec file to .labwired/components/.

const TITLES: Record<string, string> = {
  labwired_catalog: 'Catalog',
  labwired_simulate: 'Simulate',
  labwired_validate_system: 'Validate System',
  labwired_list_boards: 'List Boards',
  labwired_run_lab: 'Run Lab',
  labwired_fuzz: 'Fuzz Firmware',
  labwired_inspect_run: 'Inspect Run',
  labwired_create_session: 'Create Session',
  labwired_end_session: 'End Session',
  labwired_set_diagram: 'Set Diagram',
  labwired_set_source: 'Set Source',
  labwired_validate_diagram: 'Validate Diagram',
  labwired_search_tools: 'Search Tools',
  labwired_define_component: 'Define Component',
  labwired_ingest_svd: 'Ingest SVD',
};

export function toolTitle(name: string): string {
  return TITLES[name] ?? name
    .replace(/^labwired_/, '')
    .split('_')
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

export function decorateTools<T extends ToolLike>(tools: readonly T[]): Array<T & {
  title: string;
  annotations: {
    title: string;
    readOnlyHint: boolean;
    destructiveHint: boolean;
    openWorldHint?: boolean;
  };
}> {
  return tools.map((tool) => {
    const title = toolTitle(tool.name);
    const readOnly = READ_ONLY_TOOLS.has(tool.name);
    return {
      ...tool,
      title,
      annotations: {
        title,
        readOnlyHint: readOnly,
        destructiveHint: false,
        ...(readOnly ? {} : { openWorldHint: true }),
      },
    };
  });
}
