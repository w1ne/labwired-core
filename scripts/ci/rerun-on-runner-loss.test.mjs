import test from 'node:test';
import assert from 'node:assert/strict';
import { classify } from './rerun-on-runner-loss.mjs';

const step = (conclusion) => ({ name: 's', conclusion });
const okJob = { id: 1, name: 'lint', conclusion: 'success', steps: [step('success')] };
const failedJob = { id: 2, name: 'test', conclusion: 'failure', steps: [step('success'), step('failure')] };
const lostJob = { id: 3, name: 'build', conclusion: 'failure', steps: [step('success'), step('cancelled')] };
const noStepsJob = { id: 4, name: 'boot', conclusion: 'failure', steps: [] };
const ann = (message) => [{ title: 'Annotation', message }];
const base = { runConclusion: 'failure', runAttempt: 1 };

test('real test failure is never rerun', () => {
  assert.equal(classify({ ...base, jobs: [okJob, failedJob] }).rerun, false);
});
test('real failure beats a lost sibling', () => {
  assert.equal(classify({ ...base, jobs: [lostJob, failedJob] }).rerun, false);
});
test('runner lost: failed job with no failed step is rerun', () => {
  const v = classify({ ...base, jobs: [okJob, lostJob], annotations: { 3: ann('The self-hosted runner lost communication with the server.') } });
  assert.equal(v.rerun, true);
});
test('runner lost without annotations (zero steps) is rerun on a failure run', () => {
  assert.equal(classify({ ...base, jobs: [noStepsJob] }).rerun, true);
});
test('shutdown signal annotation is rerun', () => {
  const v = classify({ ...base, jobs: [lostJob], annotations: { 3: ann('The runner has received a shutdown signal.') } });
  assert.equal(v.rerun, true);
});
test('superseded by concurrency is not rerun', () => {
  const v = classify({
    runConclusion: 'cancelled', runAttempt: 1,
    jobs: [{ ...lostJob, conclusion: 'cancelled' }],
    annotations: { 3: ann('Canceling since a higher priority waiting request for ci-refs/heads/x exists') },
  });
  assert.equal(v.rerun, false);
});
test('cancelled run with only a bare cancel (user cancel) is not rerun', () => {
  const v = classify({
    runConclusion: 'cancelled', runAttempt: 1,
    jobs: [{ ...lostJob, conclusion: 'cancelled' }],
    annotations: { 3: ann('The operation was canceled.') },
  });
  assert.equal(v.rerun, false);
});
test('cancelled run with a lost-communication annotation is rerun', () => {
  const v = classify({
    runConclusion: 'cancelled', runAttempt: 1,
    jobs: [{ ...lostJob, conclusion: 'cancelled' }],
    annotations: { 3: ann('The runner has received a shutdown signal.') },
  });
  assert.equal(v.rerun, true);
});
test('job timeout is not rerun', () => {
  const v = classify({ ...base, jobs: [lostJob], annotations: { 3: ann('The job running on runner X has exceeded the maximum execution time of 60 minutes.') } });
  assert.equal(v.rerun, false);
});
test('attempt cap: rerun at attempts 1 and 2, not at 3', () => {
  const input = { jobs: [lostJob], annotations: { 3: ann('lost communication with the server') } };
  assert.equal(classify({ ...input, runConclusion: 'failure', runAttempt: 1 }).rerun, true);
  assert.equal(classify({ ...input, runConclusion: 'failure', runAttempt: 2 }).rerun, true);
  assert.equal(classify({ ...input, runConclusion: 'failure', runAttempt: 3 }).rerun, false);
});
test('success and no bad job are not rerun', () => {
  assert.equal(classify({ runConclusion: 'success', runAttempt: 1, jobs: [okJob] }).rerun, false);
  assert.equal(classify({ ...base, jobs: [okJob] }).rerun, false);
});
