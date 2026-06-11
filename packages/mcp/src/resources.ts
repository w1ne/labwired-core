export const AGENT_HARDWARE_LOOP_URI = 'labwired://guides/agent-hardware-loop';
export const AGENT_HARDWARE_LOOP_MIME = 'text/markdown';

const AGENT_HARDWARE_LOOP_TEXT = `# LabWired agent hardware loop

Use LabWired as a deterministic virtual hardware lab for firmware work.

1. Choose a board with \`labwired_list_boards\`.
2. Validate custom system YAML with \`labwired_validate_system\`.
3. Build or update the diagram and validate it with \`labwired_validate_diagram\`.
4. Compile firmware locally and pass the ELF as base64.
5. Run with \`labwired_run_lab\` for a preconfigured board or \`labwired_simulate\` for raw YAML control.
6. Inspect snapshots with \`labwired_inspect_run\`.
7. Use \`labwired_fuzz\` when firmware exposes the fuzz contract.
8. Iterate until serial, GPIO, cycle counts, and stop reasons match the intended behavior.

The local MCP server shells out to the \`labwired\` CLI. Set \`LABWIRED_CLI\` or \`LABWIRED_REPO_ROOT\` when running outside a checkout.
`;

export const RESOURCES = [
  {
    uri: AGENT_HARDWARE_LOOP_URI,
    name: 'labwired-agent-hardware-loop',
    mimeType: AGENT_HARDWARE_LOOP_MIME,
    description: 'Guide for using LabWired as an agent-driven virtual hardware lab.',
  },
];

export function getResource(uri: string) {
  if (uri !== AGENT_HARDWARE_LOOP_URI) return null;
  return {
    uri,
    mimeType: AGENT_HARDWARE_LOOP_MIME,
    text: AGENT_HARDWARE_LOOP_TEXT,
  };
}
