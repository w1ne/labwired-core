#!/usr/bin/env node
// Decides whether a completed workflow run died because its (self-hosted)
// runner was lost, and if so re-runs only the failed jobs via GITHUB_TOKEN.
//
// Rule (errs on the side of NOT rerunning):
//   - run conclusion must be failure or cancelled, and run_attempt < MAX_ATTEMPT
//   - jobs are read for THIS attempt; any job with a step that concluded
//     `failure` is a real failure: do nothing
//   - a job counts as runner-lost when it failed/was cancelled and has no failed
//     step (no steps at all, or the last step never finished)
//   - annotations naming concurrency supersession or a job timeout veto a rerun
//   - a run whose conclusion is `cancelled` is rerun ONLY if a job annotation
//     says the runner lost contact / received a shutdown signal; a bare
//     "The operation was canceled" is indistinguishable from a user cancel
//
// Env: GITHUB_TOKEN, GITHUB_REPOSITORY, RUN_ID, RUN_ATTEMPT, RUN_CONCLUSION.
import { pathToFileURL } from 'node:url';

export const MAX_ATTEMPT = 3; // rerun only while run_attempt < 3: at most 2 reruns
const BAD = new Set(['failure', 'cancelled']);
const LOST = ['lost communication with the server', 'runner has received a shutdown signal'];
const VETO = ['higher priority waiting request', 'exceeded the maximum execution time'];

const hasFailedStep = (job) => (job.steps ?? []).some((s) => s.conclusion === 'failure');
const text = (anns) => (anns ?? []).map((a) => `${a.title ?? ''} ${a.message ?? ''}`.toLowerCase()).join('\n');

/** Pure classifier. annotations: { [jobId]: [{title,message}] } for failed/cancelled jobs. */
export function classify({ runConclusion, runAttempt, jobs, annotations = {} }) {
  if (!BAD.has(runConclusion)) return { rerun: false, reason: `run conclusion ${runConclusion}` };
  if (!(runAttempt < MAX_ATTEMPT)) return { rerun: false, reason: `attempt cap reached (attempt ${runAttempt})` };
  const bad = jobs.filter((j) => BAD.has(j.conclusion));
  if (bad.length === 0) return { rerun: false, reason: 'no failed or cancelled job' };
  const real = bad.find(hasFailedStep);
  if (real) return { rerun: false, reason: `job "${real.name}" has a failed step` };
  const all = bad.map((j) => text(annotations[j.id]));
  const veto = VETO.find((v) => all.some((t) => t.includes(v)));
  if (veto) return { rerun: false, reason: `annotation says "${veto}"` };
  const lostSignal = LOST.find((l) => all.some((t) => t.includes(l)));
  if (runConclusion === 'cancelled' && !lostSignal) {
    return { rerun: false, reason: 'run cancelled without a runner-lost annotation (user or concurrency cancel)' };
  }
  const names = bad.map((j) => j.name).join(', ');
  return { rerun: true, reason: `runner lost (${lostSignal ?? 'no failed step'}): ${names}` };
}

async function api(path, { method = 'GET' } = {}) {
  const res = await fetch(`https://api.github.com${path}`, {
    method,
    headers: {
      authorization: `Bearer ${process.env.GITHUB_TOKEN}`,
      accept: 'application/vnd.github+json',
      'x-github-api-version': '2022-11-28',
    },
  });
  if (!res.ok) throw new Error(`${method} ${path} -> ${res.status} ${await res.text()}`);
  return res.status === 204 ? null : res.json();
}

async function main() {
  const { GITHUB_REPOSITORY: repo, RUN_ID: id } = process.env;
  const runAttempt = Number(process.env.RUN_ATTEMPT);
  const runConclusion = process.env.RUN_CONCLUSION;
  const log = (m) => console.log(`rerun-on-runner-loss: run ${id} attempt ${runAttempt}: ${m}`);
  // Cheap exits before spending any API call.
  const pre = classify({ runConclusion, runAttempt, jobs: [{ id: 0, name: '-', conclusion: 'failure', steps: [] }] });
  if (!pre.rerun) return log(`no rerun, ${pre.reason}`);

  const jobs = [];
  for (let page = 1; page <= 5; page++) {
    const r = await api(`/repos/${repo}/actions/runs/${id}/attempts/${runAttempt}/jobs?per_page=100&page=${page}`);
    jobs.push(...r.jobs);
    if (r.jobs.length < 100) break;
  }
  const bad = jobs.filter((j) => BAD.has(j.conclusion));
  const annotations = {};
  if (!bad.some(hasFailedStep)) {
    for (const j of bad) annotations[j.id] = await api(`/repos/${repo}/check-runs/${j.id}/annotations?per_page=50`);
  }
  const verdict = classify({ runConclusion, runAttempt, jobs, annotations });
  if (!verdict.rerun) return log(`no rerun, ${verdict.reason}`);
  await api(`/repos/${repo}/actions/runs/${id}/rerun-failed-jobs`, { method: 'POST' });
  log(`rerun-failed-jobs requested, ${verdict.reason}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((e) => { console.error(`rerun-on-runner-loss: ${e.message}`); process.exit(1); });
}
