/**
 * Client for the LabWired catalog API.
 * Fetches board/chip metadata from /v1/catalog for the board picker.
 */

export interface CatalogEntry {
  id: string;
  name: string;
  description: string;
  family: string;
  architecture: string;
  image_url: string;
  pass_rate: number;
  registers: number;
  verified: boolean;
  source_type: string;
}

export async function fetchCatalog(): Promise<CatalogEntry[]> {
  try {
    const res = await fetch('/v1/catalog');
    if (!res.ok) return [];
    const data = await res.json();
    return Array.isArray(data) ? data : [];
  } catch {
    return [];
  }
}

/** Extract the slug from a catalog ID like "board/nucleo-f401re" → "nucleo-f401re" */
export function catalogSlug(id: string): string {
  return id.replace(/^(board|chip|peripheral)\//, '');
}
