import { toolTitle } from './tool-metadata.js';

interface ToolLike {
  name: string;
  description: string;
  inputSchema: {
    type: string;
    properties?: Record<string, unknown>;
    required?: string[];
  };
}

export const SEARCH_TOOLS_TOOL_NAME = 'labwired_search_tools';

export const SEARCH_TOOLS_TOOL: ToolLike = {
  name: SEARCH_TOOLS_TOOL_NAME,
  description:
    'Search the LabWired MCP tool catalog by keyword and return relevant tool definitions. Use this to find board discovery, diagram validation, firmware simulation, fuzzing, snapshots, live watch session, define component, ir spec, custom device, sensor model tools.',
  inputSchema: {
    type: 'object',
    required: ['query'],
    properties: {
      query: {
        type: 'string',
        description: 'Keywords describing the LabWired capability you need, e.g. "diagram validation" or "run firmware".',
      },
      limit: {
        type: 'integer',
        minimum: 1,
        maximum: 25,
        description: 'Maximum number of tool definitions to return. Defaults to 8.',
      },
    },
  },
};

function tokenize(text: string): string[] {
  return text
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter(Boolean);
}

function toolTokens(tool: ToolLike): string[] {
  const nameTokens = tokenize(tool.name);
  const titleTokens = tokenize(toolTitle(tool.name));
  const paramTokens = Object.keys(tool.inputSchema.properties ?? {}).flatMap(tokenize);
  const descTokens = tokenize(tool.description);
  return [
    ...nameTokens,
    ...nameTokens,
    ...nameTokens,
    ...titleTokens,
    ...titleTokens,
    ...paramTokens,
    ...paramTokens,
    ...descTokens,
  ];
}

export function rankTools(query: string, tools: readonly ToolLike[], limit = 8) {
  const cap = Math.max(0, Math.min(Math.trunc(limit), tools.length));
  if (cap === 0) return [];

  const queryTerms = tokenize(query);
  if (queryTerms.length === 0) return toRanked(tools.slice(0, cap));

  const docs = tools.map((tool, idx) => {
    const tokens = toolTokens(tool);
    const tf = new Map<string, number>();
    for (const token of tokens) tf.set(token, (tf.get(token) ?? 0) + 1);
    return { tool, idx, tokens, tf };
  });
  const df = new Map<string, number>();
  for (const doc of docs) {
    for (const term of doc.tf.keys()) df.set(term, (df.get(term) ?? 0) + 1);
  }
  const avgLen = docs.reduce((sum, doc) => sum + doc.tokens.length, 0) / docs.length || 1;

  const scored = docs.map((doc) => {
    let score = 0;
    for (const term of queryTerms) {
      const frequency = doc.tf.get(term);
      if (!frequency) continue;
      const docFreq = df.get(term) ?? 0;
      const idf = Math.log(1 + (docs.length - docFreq + 0.5) / (docFreq + 0.5));
      const denom = frequency + 1.5 * (1 - 0.75 + (0.75 * doc.tokens.length) / avgLen);
      score += idf * ((frequency * 2.5) / denom);
      if (tokenize(doc.tool.name).includes(term)) score += 2;
    }
    return { ...doc, score };
  });

  scored.sort((a, b) => (b.score - a.score) || (a.idx - b.idx));
  if (scored[0]?.score === 0) return toRanked(tools.slice(0, cap));
  return toRanked(scored.slice(0, cap).map((doc) => doc.tool));
}

function toRanked(tools: readonly ToolLike[]) {
  return tools.map((tool) => ({
    name: tool.name,
    title: toolTitle(tool.name),
    description: tool.description,
    inputSchema: tool.inputSchema,
  }));
}
