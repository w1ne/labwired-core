import { ComponentDef } from '../types';
/** All available component definitions, keyed by type. */
export declare const COMPONENT_REGISTRY: Map<string, ComponentDef>;
/** Component definitions grouped by category (excludes MCU). */
export declare function getComponentsByCategory(): Record<string, ComponentDef[]>;
