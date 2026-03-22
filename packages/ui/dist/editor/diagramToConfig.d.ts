import { Diagram } from './types';
/**
 * Convert a visual diagram into system YAML + chip YAML for the WASM simulator.
 */
export declare function diagramToConfig(diagram: Diagram): {
    systemYaml: string;
    chipYaml: string;
};
