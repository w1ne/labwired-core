import { diagramToConfig, type Diagram } from '@labwired/ui';

interface ResolveRunSystemConfigArgs {
  diagram: Diagram;
  chipYaml: string;
  bundledSystemYaml: string;
  preferDiagram: boolean;
  onFallback?: (message: string) => void;
}

export function resolveRunSystemConfig({
  diagram,
  chipYaml,
  bundledSystemYaml,
  preferDiagram,
  onFallback,
}: ResolveRunSystemConfigArgs): { systemYaml: string; chipYaml: string } {
  if (preferDiagram) {
    try {
      return diagramToConfig(diagram, chipYaml);
    } catch (configErr) {
      const msg = configErr instanceof Error ? configErr.message : String(configErr);
      onFallback?.(msg);
    }
  }

  return { systemYaml: bundledSystemYaml, chipYaml };
}
