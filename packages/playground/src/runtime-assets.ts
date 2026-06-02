export function versionRuntimeAssetUrl(path: string, buildTime: number): string {
  const [base, query = ''] = path.split('?');
  const params = new URLSearchParams(query);
  params.set('v', String(buildTime));
  return `${base}?${params.toString()}`;
}
