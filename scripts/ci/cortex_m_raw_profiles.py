"""Explicit immutable raw diagnosis; never normalize counters or qualify a run."""
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import shutil

PROFILE_CAP = 1024 * 1024
SCRIPT_CAP = 256 * 1024
MANIFEST_CAP = 32768
VERSION_CAP = 8192
ANNOTATOR_SHA256 = 'e0ac7a2c69db1268d19ff52bd59e65289444f14f09c4daad1adee3ef215b6d4f'
TOOL_PATHS = dict(annotator=Path('/usr/bin/callgrind_annotate'), perl=Path('/usr/bin/perl'),
                  valgrind=Path('/usr/bin/valgrind'), dpkg=Path('/usr/bin/dpkg-query'))
VERSION_COMMANDS = (
    ('/usr/bin/dpkg-query', '-W', '-f=${Version}\n', 'valgrind'),
    ('/usr/bin/valgrind', '--version'),
    ('/usr/bin/perl', '-e', 'use Config; print "$^V\n$Config{archname}\n";'),
)


def identity(path, cap):
    path = Path(path)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        before = os.fstat(fd)
        if not stat.S_ISREG(before.st_mode) or not 0 < before.st_size <= cap:
            raise ValueError('nonregular/empty/oversized raw evidence')
        digest = hashlib.sha256()
        size = 0
        while True:
            data = os.read(fd, 65536)
            if not data: break
            size += len(data)
            if size > cap: raise ValueError('raw evidence grew beyond cap')
            digest.update(data)
        after = os.fstat(fd)
        stamp = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
        if size != before.st_size or stamp(before) != stamp(after) or stamp(after) != stamp(path.lstat()):
            raise ValueError('raw evidence changed during read')
        return dict(bytes=size, sha256=digest.hexdigest(), device=before.st_dev,
                    inode=before.st_ino, mtime_ns=before.st_mtime_ns, ctime_ns=before.st_ctime_ns)
    finally:
        os.close(fd)


def copy_exact(source, destination, cap):
    before = identity(source, cap)
    created = False
    destination_identity = None
    try:
        fd = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            with Path(destination).open('xb') as output:
                created = True
                os.fchmod(output.fileno(), 0o600)
                destination_identity = os.fstat(output.fileno())
                size = 0
                while True:
                    data = os.read(fd, 65536)
                    if not data: break
                    size += len(data)
                    if size > cap: raise ValueError('raw evidence grew beyond cap')
                    output.write(data)
                output.flush(); os.fsync(output.fileno())
        finally:
            os.close(fd)
        preserved = identity(destination, cap)
        after = identity(source, cap)
        if before != after or (preserved['bytes'],preserved['sha256']) != (before['bytes'],before['sha256']):
            raise ValueError('raw evidence changed during exact copy')
        Path(destination).chmod(0o444)
        return dict(path=str(Path(destination).resolve()), bytes=preserved['bytes'], sha256=preserved['sha256'])
    except Exception:
        if created:
            current = Path(destination).lstat()
            if (current.st_dev,current.st_ino) == (destination_identity.st_dev,destination_identity.st_ino):
                Path(destination).unlink()
        raise


class RawProfiles:
    def __init__(self, output, budget, collect, provenance=None):
        from cortex_m_perf_evidence import Budget
        from cortex_m_perf_diagnostics import reserve
        # These allocations cover all three slots before any profiling child.
        allocation = 3 * PROFILE_CAP + SCRIPT_CAP + MANIFEST_CAP + len(VERSION_COMMANDS) * VERSION_CAP
        reserve(budget, allocation)
        self.directory = Path(output) / 'raw-l476-repeats'
        self.directory.mkdir(mode=0o700)
        self.manifest = self.directory / 'manifest.json'
        self.manifest_hash = None
        self.report = dict(protocol='immutable-base-l476-200k-batch-v1', qualified=False, complete=False,
            profiles=[], reserved_bytes=allocation, profile_cap_bytes=PROFILE_CAP,
            shared_retention_budget_bytes=5*1024*1024, tool={}, failure_reason=None,
            provenance=provenance)
        self.checkpoint()
        try:
            originals = {name:identity(path, SCRIPT_CAP if name=='annotator' else 32*1024*1024)
                         for name,path in TOOL_PATHS.items()}
            if originals['annotator']['sha256'] != ANNOTATOR_SHA256:
                raise ValueError('official annotator bytes do not match pinned provenance')
            script = copy_exact(TOOL_PATHS['annotator'], self.directory/'callgrind_annotate', SCRIPT_CAP)
            versions = []
            version_budget = Budget(len(VERSION_COMMANDS)*VERSION_CAP)
            for command in VERSION_COMMANDS:
                result = collect(list(command), timeout=5, stream_limit=VERSION_CAP, budget=version_budget)
                if result.returncode != 0 or result.producer_exit_code != 0 or result.stderr or len(result.stdout)>VERSION_CAP:
                    raise ValueError('fixed official version probe failed')
                versions.append(result.stdout.decode('utf-8').strip())
            if versions[:2] != ['1:3.22.0-0ubuntu3','valgrind-3.22.0'] or not re.fullmatch(r'v5\.\d+\.\d+\n[\w.-]+',versions[2]):
                raise ValueError('official package/tool/interpreter version mismatch')
            if originals != {name:identity(path,SCRIPT_CAP if name=='annotator' else 32*1024*1024) for name,path in TOOL_PATHS.items()}:
                raise ValueError('official tool changed during provenance capture')
            with Path(script['path']).open('rb') as stream:
                first_line=stream.readline(129)
            self.report['tool'] = dict(annotator=script, original_paths={k:str(v) for k,v in TOOL_PATHS.items()},
                resolved_paths={k:str(v.resolve()) for k,v in TOOL_PATHS.items()},
                source_modes={k:stat.S_IMODE(v.stat().st_mode) for k,v in TOOL_PATHS.items()},
                annotator_shebang=first_line.decode('utf-8').strip() if first_line.startswith(b'#!') and len(first_line)<=128 else None,
                originals=originals, package_version=versions[0], valgrind_version=versions[1],
                perl_version_arch=versions[2], version_commands=[list(c) for c in VERSION_COMMANDS],
                script_version_source='pinned script bytes and official package; no annotation execution')
            self.checkpoint()
        except Exception as error:
            self.fail(error)
            raise

    def checkpoint(self):
        data=(json.dumps(self.report,indent=2,allow_nan=False)+'\n').encode()
        if len(data)>MANIFEST_CAP: raise ValueError('raw profile manifest exceeds reserved cap')
        if self.manifest_hash is not None and hashlib.sha256(self.manifest.read_bytes()).hexdigest()!=self.manifest_hash:
            raise ValueError('raw profile manifest changed outside owner')
        temporary=self.directory/'manifest.next'
        with temporary.open('xb') as stream:
            os.fchmod(stream.fileno(),0o600);stream.write(data);stream.flush();os.fsync(stream.fileno())
        os.replace(temporary,self.manifest)
        self.manifest_hash=hashlib.sha256(data).hexdigest()

    def fail(self,error):
        self.report['complete']=False
        self.report['failure_reason']=str(error)[:1024]
        self.checkpoint()

    def preflight(self, command, environment):
        if command[0]!='valgrind':raise ValueError('unexpected raw-mode native command')
        resolved=shutil.which('valgrind',path=environment.get('PATH'))
        if resolved is None or Path(resolved).resolve()!=TOOL_PATHS['valgrind'].resolve():
            raise ValueError('PATH substituted the pinned Valgrind executable')
        if identity(TOOL_PATHS['valgrind'],32*1024*1024)!=self.report['tool']['originals']['valgrind']:
            raise ValueError('pinned Valgrind executable changed')

    def capture(self, command, row):
        selected=(row['side'],row['board'],row['mode'],row['steps'])==('base','stm32l476','batch',200000)
        if not selected: return None
        try:
            repeat=row['repeat']
            if type(repeat) is not int or repeat not in (0,1,2) or any(p['repeat']==repeat for p in self.report['profiles']):
                raise ValueError('duplicate/unexpected raw profile repeat')
            paths=[x.split('=',1)[1] for x in command if x.startswith('--callgrind-out-file=')]
            if len(paths)!=1 or command!=row['argv'] or row['exit_code']!=0 or row['producer_exit_code']!=0:
                raise ValueError('ambiguous/failed raw profile invocation')
            source=Path(paths[0])
            original=identity(source,PROFILE_CAP)
            with source.open('rb') as stream: data=stream.read(PROFILE_CAP+1)
            text=data.decode('utf-8')
            target=' '.join(command[command.index('run')-1:])
            def field(name): return re.findall(r'^'+name+r':\s*([^\r\n]+)$',text,re.M)
            if field('version')!=['1'] or field('creator')!=['callgrind-3.22.0'] or field('cmd')!=[target] or field('events')!=['Ir'] or field('summary')!=[str(row['ir'])]:
                raise ValueError('raw profile header/command/events/summary mismatch')
            totals=field('totals')
            if totals and totals!=[str(row['ir'])]:raise ValueError('raw profile totals mismatch')
            destination=self.directory/f'base-stm32l476-batch-200000-r{repeat}.callgrind'
            proof=copy_exact(source,destination,PROFILE_CAP)
            if identity(source,PROFILE_CAP)!=original or (proof['bytes'],proof['sha256'])!=(original['bytes'],original['sha256']):
                raise ValueError('raw profile changed before copy')
            proof.update(repeat=repeat,raw_invocation=dict(row),source_identity=original,
                         source_path=str(source),source_preserved=True)
            self.report['profiles'].append(proof)
            self.checkpoint()
            return proof
        except Exception as error:
            self.fail(error)
            raise

    def verify(self):
        for name,path in TOOL_PATHS.items():
            if identity(path,SCRIPT_CAP if name=='annotator' else 32*1024*1024)!=self.report['tool']['originals'][name]:
                raise ValueError('pinned runtime/tool identity changed')
        for proof in self.report['profiles']+[self.report['tool']['annotator']]:
            actual=identity(proof['path'],SCRIPT_CAP if proof is self.report['tool']['annotator'] else PROFILE_CAP)
            if (actual['bytes'],actual['sha256'])!=(proof['bytes'],proof['sha256']):
                raise ValueError('retained raw evidence hash changed')
        if len(self.manifest.read_bytes())>MANIFEST_CAP or hashlib.sha256(self.manifest.read_bytes()).hexdigest()!=self.manifest_hash:
            raise ValueError('raw profile manifest hash changed')

    def artifact_bytes(self, final_receipt_bytes):
        total=final_receipt_bytes
        output=self.directory.parent
        for directory in (output,output.parent/'perf-build-evidence'):
            if not directory.exists():continue
            if directory.is_symlink():raise ValueError('artifact directory is a symlink')
            for path in directory.rglob('*'):
                mode=path.lstat().st_mode
                if stat.S_ISDIR(mode):continue
                if not stat.S_ISREG(mode):raise ValueError('nonregular artifact path')
                if path==output/'qualification.json':continue
                total+=path.stat().st_size
                if total>8*1024*1024:raise ValueError('raw diagnostic artifact exceeds fixed8MiB cap')
        return total

    def finish(self):
        try:
            self.verify()
            if sorted(p['repeat'] for p in self.report['profiles']) != [0,1,2]:
                raise ValueError('incomplete selected raw profile inventory')
            self.report['complete']=True
            self.checkpoint()
            return self.report
        except Exception as error:
            self.fail(error)
            raise
