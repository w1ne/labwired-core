import type { BoardIoKind } from './types';
import { CATALOG } from './catalog';

export interface ComponentMeta { boardIoKind?: BoardIoKind; }

/** Derived from the catalog; kept for backward compatibility. */
export const COMPONENT_META: Record<string, ComponentMeta> = Object.fromEntries(
  Object.entries(CATALOG).map(([k, v]) => [
    k,
    v.boardIoKind ? { boardIoKind: v.boardIoKind } : {},
  ]),
);
