export const AGENT_HARDWARE_LOOP_URI = 'labwired://guides/agent-hardware-loop';
export const AGENT_HARDWARE_LOOP_NAME = 'labwired-agent-hardware-loop';
export const AGENT_HARDWARE_LOOP_MIME = 'text/markdown';

const AGENT_HARDWARE_LOOP_TEXT = `# LabWired agent hardware loop

Use LabWired as a deterministic virtual hardware lab for firmware work.

1. Choose a board with \`labwired_list_boards\`.
2. Discover modeled peripherals with \`labwired_list_components\`.
3. Build or update the diagram with an MCU, components, and wires.
4. Validate the diagram with \`labwired_validate_diagram\` before running.
5. Compile firmware outside hosted MCP using the documented scaffold and target flags.
6. Run the ELF with \`labwired_run\`.
7. Inspect serial output, cycle counts, stop reasons, and hardware diagnosis.
8. Iterate on firmware or wiring until simulator evidence matches the intended behavior.

Hosted MCP accepts Clerk OAuth bearer tokens and \`lwk_live_\` workspace API keys. The hosted connector runs firmware through the LabWired builder; it does not compile source.
`;

export interface McpResourceDescriptor {
  uri: string;
  name: string;
  mimeType: string;
  description: string;
}

export interface McpResourceContent {
  uri: string;
  mimeType: string;
  text: string;
}

export const RESOURCES: readonly McpResourceDescriptor[] = [
  {
    uri: AGENT_HARDWARE_LOOP_URI,
    name: AGENT_HARDWARE_LOOP_NAME,
    mimeType: AGENT_HARDWARE_LOOP_MIME,
    description: 'Guide for using LabWired as an agent-driven virtual hardware lab.',
  },
];

export function getResource(uri: string): McpResourceContent | null {
  if (uri !== AGENT_HARDWARE_LOOP_URI) return null;
  return {
    uri,
    mimeType: AGENT_HARDWARE_LOOP_MIME,
    text: AGENT_HARDWARE_LOOP_TEXT,
  };
}
