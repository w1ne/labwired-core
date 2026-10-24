import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {admitRequest, admitManifest, collectFiles, REQUEST_SHA, HEX_SHA} from './evidence.mjs';

test('wrong request identity fails rather than accepting a source default', () => {
    assert.throws(() => admitRequest({sha: REQUEST_SHA, request: {replaceFiles: {'/codal.json': '{}'}}}), /identity mismatch/);
});
test('CDN or mutated bytes cannot stand in for the from-source HEX', () => {
    const entry = {variant: 'samd51adafruit', sha: REQUEST_SHA, file: `${REQUEST_SHA}.hex`, sha256: HEX_SHA, bytes: 376911};
    assert.throws(() => admitManifest(entry, Buffer.alloc(376911)), /identity mismatch/);
    assert.throws(() => admitManifest({...entry, variant: 'samd51'}, Buffer.alloc(0)), /identity mismatch/);
});
test('selected regular files remain exact and second capture refuses overwrite', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'pybadge-evidence-control-'));
    try {
        const input = path.join(root, 'input'), out = path.join(root, 'out');
        fs.mkdirSync(input);
        const data = Buffer.from('CXX_DEFINES = -DUSE_RGB444=1\r\n');
        fs.writeFileSync(path.join(input, 'flags.make'), data);
        fs.writeFileSync(path.join(input, 'ignored.o'), 'not evidence');
        const records = collectFiles(input, out, name => name === 'flags.make');
        assert.equal(records.length, 1);
        assert.deepEqual(fs.readFileSync(path.join(out, 'flags.make')), data);
        assert.throws(() => collectFiles(input, out, () => true), /EEXIST/);
    } finally { fs.rmSync(root, {recursive: true}); }
});
test('selected symlinks and oversized captures are rejected', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'pybadge-evidence-control-'));
    try {
        const input = path.join(root, 'input'); fs.mkdirSync(input);
        fs.writeFileSync(path.join(input, 'real'), '1234');
        fs.symlinkSync('real', path.join(input, 'link'));
        assert.throws(() => collectFiles(input, path.join(root, 'out'), n => n === 'link'), /selected symlink/);
        assert.throws(() => collectFiles(input, path.join(root, 'out'), n => n === 'real', 3), /byte limit/);
    } finally { fs.rmSync(root, {recursive: true}); }
});
