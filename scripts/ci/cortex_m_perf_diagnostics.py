"""Opt-in common-input and official Callgrind annotation evidence.

No normalization, changed counters, simulator defaults or native profiling mode.
"""

from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import sys

MAX_ELF_BYTES = 512 * 1024
MAX_CALLGRIND_BYTES = 64 * 1024 * 1024
MAX_ANNOTATION_BYTES = 256 * 1024
FIXTURES = frozenset(("nrf", "stm32"))


def inspect_elf(path):
    """Describe ELF32 ARM entry and every PT_LOAD without altering file bytes."""
    path = Path(path)
    if not path.is_file() or not 84 <= path.stat().st_size <= MAX_ELF_BYTES:
        raise ValueError("missing/oversized/truncated diagnostic ELF")
    with path.open('rb') as stream:
        data = stream.read(MAX_ELF_BYTES+1)
    if len(data) > MAX_ELF_BYTES or data[:7] != b"\x7fELF\x01\x01\x01":
        raise ValueError("diagnostic requires ELF32 little-endian version1")
    header = struct.unpack_from("<HHIIIIIHHHHHH", data, 16)
    kind, machine, version, entry, phoff, _, flags, ehsize, phsize, phnum, *_ = header
    if (kind, machine, version, ehsize, phsize) != (2, 40, 1, 52, 32) or not 1 <= phnum <= 64 or phoff < 52 or phoff + phnum * phsize > len(data):
        raise ValueError("invalid ARM executable program-header table")
    segments = []
    for index in range(phnum):
        ptype, offset, vaddr, paddr, filesz, memsz, permissions, align = struct.unpack_from("<IIIIIIII", data, phoff + index * phsize)
        if ptype != 1:
            continue
        if not 0 <= filesz <= memsz <= 16 * 1024 * 1024 or memsz == 0 or offset + filesz > len(data) or permissions & ~7:
            raise ValueError("invalid load content/zero-fill/permissions")
        if vaddr + memsz > 2**32 or paddr + memsz > 2**32 or (align not in (0,1) and (align & (align-1) or vaddr % align != offset % align)):
            raise ValueError("invalid load address/alignment")
        if any(max(vaddr,s["vaddr"]) < min(vaddr+memsz,s["vaddr"]+s["memsz"])
               or max(paddr,s["paddr"]) < min(paddr+memsz,s["paddr"]+s["memsz"]) for s in segments):
            raise ValueError("overlapping diagnostic load segments")
        segments.append(dict(vaddr=vaddr,paddr=paddr,filesz=filesz,memsz=memsz,
            zero_fill=memsz-filesz,permissions=permissions,align=align,
            content_sha256=hashlib.sha256(data[offset:offset+filesz]).hexdigest()))
    if not segments or not any(s["permissions"] & 1 and s["vaddr"] <= (entry & ~1) < s["vaddr"]+s["filesz"] for s in segments):
        raise ValueError("no load-backed executable entry")
    return dict(path=str(path.resolve()),bytes=len(data),sha256=hashlib.sha256(data).hexdigest(),
                image=dict(entry=entry,flags=flags,segments=segments))


def reserve(budget, amount):
    if amount > budget.remaining:
        raise ValueError("diagnostic retained evidence exceeds shared byte budget")
    budget.remaining -= amount


def frozen_copy(source, destination, budget):
    proof = inspect_elf(source)
    with Path(source).open('rb') as stream:
        data=stream.read(MAX_ELF_BYTES+1)
    if hashlib.sha256(data).hexdigest() != proof["sha256"]:
        raise ValueError("fixture changed during preservation")
    reserve(budget,len(data))
    destination.parent.mkdir(exist_ok=True)
    with destination.open("xb") as stream:
        stream.write(data)
        os.fchmod(stream.fileno(),0o444)
    preserved = inspect_elf(destination)
    if preserved["sha256"] != proof["sha256"]:
        raise ValueError("preserved fixture differs from original")
    return dict(original=proof,preserved=preserved)


@contextmanager
def builder_scope(module, source, runner):
    previous = module.subprocess,os.getcwd(),sys.argv[:],dict(os.environ)
    try:
        module.subprocess=runner
        os.chdir(source)
        os.environ.pop("LABWIRED_ARM_SINGLE_STEP",None)
        yield
    finally:
        module.subprocess=previous[0]
        os.chdir(previous[1]);sys.argv=previous[2]
        os.environ.clear();os.environ.update(previous[3])


class CommonFixtures:
    def __init__(self, paths, evidence):
        self.paths,self.evidence=paths,evidence

    def verify(self, path):
        path=Path(path).resolve()
        name=next((n for n,p in self.paths.items() if p==path),None)
        if name is None or path.stat().st_mode & 0o222:
            raise ValueError("firmware is not the exclusive readonly common fixture")
        proof=inspect_elf(path)
        if proof != self.evidence["common"][name]:
            raise ValueError("common whole-file fixture changed")
        return proof

    @contextmanager
    def install(self,module):
        original=module.build_fixtures
        def use_common(names):
            if set(names) != FIXTURES:
                raise ValueError("unexpected common fixture request")
            for path in self.paths.values():self.verify(path)
            return dict(self.paths),{}
        try:
            module.build_fixtures=use_common
            yield
        finally:
            module.build_fixtures=original


def prepare_common_fixtures(modules,sources,output,budget,runner_type):
    output=Path(output);folder=output/'fixtures';folder.mkdir(exist_ok=False)
    build_output=output/'fixture-build';build_output.mkdir(exist_ok=False)
    # Reserve the bounded provenance even if a later producer fills the budget.
    reserve(budget,65536)
    evidence=dict(originals={},preserved={},common={},build_commands={})
    provenance=output/'fixture-provenance.json'
    try:
        for side in ('base','head'):
            module=modules[side]
            covered,_=module.plan_coverage(module.discover_chips())
            if {covered.get(b) for b in ('nrf52840','stm32f405','stm32l476')} != FIXTURES:
                raise ValueError("selected boards do not derive the required memory-map pair")
            for name,flash in (('nrf',0),('stm32',0x08000000)):
                if module.fixture_origins(name) != (flash,0x20000000):
                    raise ValueError("changed fixture memory-map origins")
            runner=runner_type(module,side,build_output,[],budget)
            commands=[]
            class BuildRecorder:
                def run(self,command,**kwargs):
                    env=kwargs.get('env') if kwargs.get('env') is not None else os.environ
                    commands.append(dict(argv=list(command),cwd=str(Path(kwargs.get('cwd') or os.getcwd()).resolve()),
                        selected_environment={key:env.get(key) for key in ('RUSTUP_TOOLCHAIN','LABWIRED_PERF_FLASH_ORIGIN','LABWIRED_PERF_RAM_ORIGIN')}))
                    return runner.run(command,**kwargs)
            evidence['build_commands'][side]=commands
            with builder_scope(module,sources[side],BuildRecorder()):
                built,skipped=module.build_fixtures(set(FIXTURES))
            if skipped or set(built) != FIXTURES:
                raise ValueError("missing/extra/skipped original diagnostic fixtures")
            evidence['originals'][side]={};evidence['preserved'][side]={}
            for name in sorted(FIXTURES):
                copy=frozen_copy(built[name],folder/side/(name+'.elf'),budget)
                evidence['originals'][side][name]=copy['original']
                evidence['preserved'][side][name]=copy['preserved']
        for name in FIXTURES:
            if evidence['originals']['base'][name]['image'] != evidence['originals']['head'][name]['image']:
                raise ValueError("independently built ELF entry/PT_LOAD images differ")
        paths={}
        for name in sorted(FIXTURES):
            path=(folder/'common'/(name+'.elf')).resolve()
            copy=frozen_copy(evidence['preserved']['base'][name]['path'],path,budget)
            paths[name]=path;evidence['common'][name]=copy['preserved']
        return CommonFixtures(paths,evidence)
    finally:
        data=json.dumps(evidence,indent=2)+'\n'
        if len(data.encode())>65536:raise ValueError("oversized fixture provenance")
        provenance.write_text(data)


def parse_annotation(data, raw_ir):
    if not data or len(data)>MAX_ANNOTATION_BYTES:
        raise ValueError("empty/oversized Callgrind annotation")
    text=data.decode('utf-8')
    if not re.search(r'^Events shown:\s+Ir\s*$',text,re.M):
        raise ValueError("annotation does not show the exact Ir event")
    number=r'(?:\d{1,3}(?:,\d{3})+|\d+)'
    totals=re.findall(r'^\s*('+number+r')\s+PROGRAM TOTALS\s*$',text,re.M)
    if len(totals)!=1 or int(totals[0].replace(',','')) != raw_ir:
        raise ValueError("missing/calculated/inconsistent official PROGRAM TOTALS")
    sections=re.split(r'^\s*Ir\s+file:function\s*$',text,flags=re.M)
    if len(sections)!=2:raise ValueError("missing/duplicate self-cost function table")
    functions=[];names=set()
    for line in sections[1].splitlines():
        if not line.strip() or re.fullmatch(r'-+',line.strip()):continue
        match=re.fullmatch(r'\s*('+number+r'|\.)\s+(\S[^\r\n]*:[^\r\n]+)\s*',line)
        if not match or len(match[2].encode())>2048 or match[2] in names or len(functions)>=4096:
            raise ValueError("unsupported/duplicate/oversized function-cost row")
        names.add(match[2]);functions.append(dict(ir=0 if match[1]=='.' else int(match[1].replace(',','')),function=match[2]))
    total=sum(row['ir'] for row in functions)
    if not functions or total!=raw_ir or raw_ir<=0:
        raise ValueError("function self-costs do not account for unchanged aggregate Ir")
    return dict(total_ir=raw_ir,self_cost_sum=total,functions=functions)


def annotate_call(command,raw_ir,prefix,collect,budget,sha256):
    paths=[arg.split('=',1)[1] for arg in command if arg.startswith('--callgrind-out-file=')]
    if len(paths)!=1:raise ValueError("missing/ambiguous original Callgrind output path")
    path=Path(paths[0])
    if not path.is_file() or not 1<=path.stat().st_size<=MAX_CALLGRIND_BYTES:
        raise ValueError("missing/oversized original Callgrind input")
    executable=shutil.which('callgrind_annotate')
    if executable is None:raise ValueError("official callgrind_annotate unavailable")
    argv=[executable,'--show=Ir','--sort=Ir','--threshold=100','--show-percs=no',
          '--auto=no','--inclusive=no','--tree=none',str(path)]
    reserve(budget,8192)
    before=sha256(path)
    result=collect(argv,timeout=30,stream_limit=MAX_ANNOTATION_BYTES,budget=budget)
    Path(str(prefix)+'.callgrind.stdout').write_bytes(result.stdout)
    Path(str(prefix)+'.callgrind.stderr').write_bytes(result.stderr)
    metadata=dict(input_sha256=before,input_bytes=path.stat().st_size,argv=argv,
                  annotator_sha256=sha256(executable),exit_code=result.returncode,
                  producer_exit_code=result.producer_exit_code)
    data=json.dumps(metadata)+'\n'
    if len(data.encode())>8192:raise ValueError('oversized annotation metadata')
    Path(str(prefix)+'.callgrind.json').write_text(data)
    if result.returncode!=0 or result.stderr or sha256(path)!=before:
        raise ValueError("official Callgrind annotation failed/warned or input changed")
    proof=parse_annotation(result.stdout,raw_ir)
    proof['function_count']=len(proof.pop('functions'))
    proof.update(metadata,log_prefix=Path(prefix).name,
                 annotation_sha256=hashlib.sha256(result.stdout).hexdigest())
    return proof
