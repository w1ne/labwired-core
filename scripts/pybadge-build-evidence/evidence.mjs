// Qualification orchestration only. Original request generation/build stays in
// the immutable consumer checkout; no worker or runtime implementation is copied.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {execFileSync} from 'node:child_process';
import {pathToFileURL, fileURLToPath} from 'node:url';

export const LITE_PIN = '6a7027e117a6698866516f52feca259127c80c7d';
export const BUILDER_BLOB = 'cff2602627a48cc3089c8e9a06efe2211de4a040';
export const REQUEST_SHA = '19efcdc73769fdfdeb51aa215c528bebad59782cbc538f72f4a194326f1f42b1';
export const HEX_SHA = '9c2310bd5a65f0543c69a076e51a4067c803202a39228f3a7de41451be0da9ca';
export const sha256 = data => crypto.createHash('sha256').update(data).digest('hex');

export function admitRequest(result) {
    const raw = Buffer.from(JSON.stringify(result.request));
    if (result.sha !== REQUEST_SHA || sha256(raw) !== REQUEST_SHA) {
        throw new Error('request identity mismatch; do not normalize or re-pin');
    }
    if (!result.request.replaceFiles || !result.request.replaceFiles['/codal.json']) {
        throw new Error('missing original codal configuration');
    }
    return raw;
}

export function admitManifest(entry, hex) {
    if (!entry || entry.variant !== 'samd51adafruit' || entry.sha !== REQUEST_SHA ||
        entry.file !== `${REQUEST_SHA}.hex` || entry.sha256 !== HEX_SHA ||
        entry.bytes !== 376911 || hex.length !== entry.bytes || sha256(hex) !== HEX_SHA) {
        throw new Error('from-source HEX identity mismatch; preserve failure');
    }
}

// Copy only bounded regular files. Paths in the receipt are repository-relative;
// raw generated files remain byte-preserved, not rewritten to imply compilation.
export function collectFiles(root, out, select, limit = 8 * 1024 * 1024) {
    const records = [];
    let bytes = 0;
    function walk(dir) {
        for (const entry of fs.readdirSync(dir, {withFileTypes: true}).sort((a, b) => a.name.localeCompare(b.name))) {
            if (entry.name === '.git') continue;
            const absolute = path.join(dir, entry.name);
            const relative = path.relative(root, absolute).split(path.sep).join('/');
            if (entry.isSymbolicLink()) {
                if (select(relative)) throw new Error(`selected symlink: ${relative}`);
                continue;
            }
            if (entry.isDirectory()) { walk(absolute); continue; }
            if (!entry.isFile() || !select(relative)) continue;
            const size = fs.statSync(absolute).size;
            bytes += size;
            if (bytes > limit) throw new Error('evidence byte limit exceeded');
            const data = fs.readFileSync(absolute);
            const dest = path.join(out, relative);
            fs.mkdirSync(path.dirname(dest), {recursive: true});
            fs.writeFileSync(dest, data, {flag: 'wx'});
            records.push({path: relative, bytes: data.length, sha256: sha256(data)});
        }
    }
    walk(root);
    return records;
}

async function main() {
    const [mode, liteArg, workArg, outArg] = process.argv.slice(2);
    if (!['request', 'collect'].includes(mode) || !liteArg || !workArg || !outArg) {
        throw new Error('usage: evidence.mjs request|collect LITE WORK OUT');
    }
    const lite = path.resolve(liteArg), work = path.resolve(workArg), out = path.resolve(outArg);
    const git = args => execFileSync('git', ['-C', lite, ...args], {encoding: 'utf8'}).trim();
    if (git(['rev-parse', 'HEAD']) !== LITE_PIN ||
        git(['hash-object', 'scripts/build-makecode-arcade-bases.mjs']) !== BUILDER_BLOB) {
        throw new Error('consumer source identity mismatch');
    }
    fs.mkdirSync(out, {recursive: true});
    if (mode === 'request') {
        for (const name of ['LICENSE-pxt-arcade.txt', 'LICENSE-pxt-core.txt']) {
            fs.copyFileSync(path.join(lite, 'packages/scratch-gui/static/makecode/arcade', name), path.join(out, name), fs.constants.COPYFILE_EXCL);
        }
        const {buildRequest} = await import(pathToFileURL(path.join(lite, 'scripts/build-makecode-arcade-bases.mjs')));
        const result = await buildRequest('samd51adafruit');
        const raw = admitRequest(result);
        fs.writeFileSync(path.join(out, 'request.json'), raw, {flag: 'wx'});
        fs.writeFileSync(path.join(out, 'request-evidence.json'), JSON.stringify({
            schema: 1, consumer: LITE_PIN, builderBlob: BUILDER_BLOB,
            requestSha256: sha256(raw), compileService: result.compileService,
            boundary: 'Original offline worker request; not compiler or loaded CF2 proof'
        }, null, 2) + '\n', {flag: 'wx'});
        console.log('Original request identity PASS');
        return;
    }
    const manifest = fs.readFileSync(path.join(work, 'bases/manifest.json'));
    const entry = JSON.parse(manifest).samd51adafruit;
    const hex = fs.readFileSync(path.join(work, 'bases', `${REQUEST_SHA}.hex`));
    admitManifest(entry, hex);
    fs.writeFileSync(path.join(out, 'builder-manifest.json'), manifest, {flag: 'wx'});
    const root = path.join(work, 'samd51adafruit');
    const files = collectFiles(root, path.join(out, 'generated'), name =>
        /(?:^|\/)(?:flags\.make|CMakeCache\.txt|CMakeCXXCompiler\.cmake|compile_commands\.json|codal\.json)$/.test(name) ||
        /(?:config|defines|definitions).*\.h$/i.test(path.basename(name)) ||
        /(?:^|\/)(?:LICENSE|LICENCE)(?:\.[^/]*)?$/i.test(name));
    if (!files.some(f => f.path.endsWith('/flags.make')) ||
        !files.some(f => f.path.endsWith('CMakeCache.txt')) ||
        !files.some(f => f.path === 'build/codal_extra_definitions.h')) {
        throw new Error('missing generated compiler configuration');
    }
    fs.writeFileSync(path.join(out, 'build-evidence.json'), JSON.stringify({
        schema: 1, consumer: LITE_PIN, builderBlob: BUILDER_BLOB, entry, files,
        boundary: 'Actual clean build and generated configuration; no loaded CF2, constructor, guest or panel proof'
    }, null, 2) + '\n', {flag: 'wx'});
    console.log(`From-source build identity PASS; ${files.length} generated/configuration/notice files retained`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
