import type { Env } from '../types.js';

export interface BuilderRunDiagnosis {
  summary: string;
  faulting_pc?: string;
  symbol?: string;
  last_instructions?: string[];
  hint?: string;
}

export interface BuilderRunResult {
  status: string;
  stopReason: string;
  stepsExecuted: number;
  cycles: number;
  instructions: number;
  serial: string;
  peripherals: { id: string; type: string; state: unknown }[];
  timedOut: boolean;
  diagnosis?: BuilderRunDiagnosis;
}

async function post<T>(env: Env, path: string, body: unknown): Promise<T> {
  const resp = await fetch(`${env.BUILDER_URL}${path}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-builder-secret': env.BUILDER_SECRET },
    body: JSON.stringify(body),
  });
  if (!resp.ok) throw new Error(`builder ${path} → ${resp.status}`);
  return resp.json() as Promise<T>;
}

export function builderRun(
  env: Env,
  req: { elfBase64: string; systemYaml: string; chipYaml: string; maxSteps: number },
) {
  return post<BuilderRunResult>(env, '/run', req);
}
