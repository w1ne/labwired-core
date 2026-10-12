#!/usr/bin/env python3
"""Prepare the distinct, pinned RV1103 Linux basket profile (guest NOT_RUN)."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import struct
import tempfile

def load_script(name, filename):
    spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name(filename))
    module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module); return module

modules=load_script('rv1103_modules','prepare-rv1103-modules.py')
base=modules.base
FIXTURE=base.EXAMPLE/'basket'
ARCHIVE_LIMIT=8*1024*1024
SOURCE_LIMIT=65536


def read_regular(path, limit):
    path=Path(path)
    if path.is_symlink() or any(p.is_symlink() for p in path.parents) or not path.is_file():
        raise ValueError('nonregular or symlink input path')
    with path.open('rb') as stream: data=stream.read(limit+1)
    if len(data)>limit: raise ValueError('input byte bound exceeded')
    return data


def checked(path, expected, label):
    data=read_regular(path,expected['bytes'])
    modules.require_identity(data,(expected['bytes'],expected['sha256']),label)
    return data


def tool_identity(name):
    resolved=shutil.which(name)
    if resolved is None: raise ValueError('missing tool '+name)
    path=Path(resolved).resolve()
    # Guard the executable itself before executing it or allocating its hash.
    binary=read_regular(path,64*1024*1024)
    command=[str(path),'--version']
    environment={'PATH':os.environ.get('PATH','/usr/bin:/bin'),'LC_ALL':'C','TZ':'UTC'}
    header_bytes=len((json.dumps(command)+'\n').encode())
    with tempfile.NamedTemporaryFile(mode='w+b',prefix='.rv1103-basket-tool-') as log:
        modules.run_command(command,path.parent,environment,log,timeout=modules.TOOL_VERSION_TIMEOUT,
                            log_limit=header_bytes+modules.TOOL_VERSION_LIMIT)
        log.seek(header_bytes); output=log.read(modules.TOOL_VERSION_LIMIT+1)
    if len(output)>modules.TOOL_VERSION_LIMIT: raise ValueError('tool version byte bound')
    return dict(path=str(path),version=output.decode('utf-8'),**modules.identity(binary),version_command=command,
                version_output=modules.identity(output),version_capture='combined stdout/stderr',
                version_timeout_seconds=modules.TOOL_VERSION_TIMEOUT,version_byte_limit=modules.TOOL_VERSION_LIMIT,
                binary_byte_limit=64*1024*1024,version_environment=environment)


def load_pins():
    pins=json.loads(read_regular(FIXTURE/'pins.json',SOURCE_LIMIT))
    if pins['schema']!=1 or pins['profile']!='rv1103-linux-basket-v1': raise ValueError('pin schema/profile')
    return pins


def fixture_sources(pins, directory=FIXTURE):
    return {name:checked(Path(directory)/name,pin,name) for name,pin in pins['sources'].items()}


def phase_header(meta):
    if meta['schema']!=1 or meta['profile']!='rv1103-linux-basket-v1': raise ValueError('phase schema/profile')
    if len(meta['phases'])!=4 or [p['sequence'] for p in meta['phases']]!=[1,2,3,4]: raise ValueError('phase order')
    lines=['/* Generated only from reviewed shared phases.json. */',
           f'#define READY_WAIT_MS {meta["ready_wait_ms"]}',f'#define UART_DEADLINE_MS {meta["uart_deadline_ms"]}',
           f'#define I2C_ADDRESS {meta["address"]}',f'#define DEVID {meta["devid"]}',f'#define AXIS_REGISTER {meta["axis_register"]}',
           f'#define OFF_ERRNO {meta["phases"][2]["expected_i2c_errno"]}',
           f'#define CONFIG_COUNT {len(meta["configuration"])}',
           'static const unsigned char configuration[][2] = {'+', '.join('{'+str(p['register'])+', '+str(p['value'])+'}' for p in meta['configuration'])+'};',
           f'#define PHASE_COUNT {len(meta["phases"])}','static const struct { const char *id; int powered; short raw[3]; } phases[] = {']
    for phase in meta['phases']:
        raw=phase.get('expected_raw',[0,0,0])
        lines.append('{'+json.dumps(phase['id'])+', '+str(int(phase['external_power']))+', {'+', '.join(map(str,raw))+'}},')
    lines+=['};',f'#define UART_EXCHANGE_COUNT {len(meta["uart_exchanges"])}','static const struct { const char *command, *reply; } exchanges[] = {']
    for exchange in meta['uart_exchanges']:
        lines.append('{'+json.dumps(exchange['command'])+', '+json.dumps(exchange['reply'])+'},')
    lines+=['};','']
    return '\n'.join(lines).encode('ascii')


def verify_helper_elf(data):
    if len(data)<52 or len(data)>131072: raise ValueError('helper ELF byte bound')
    h=struct.unpack_from('<16sHHIIIIIHHHHHH',data)
    ident,kind,machine,version,entry,phoff,shoff,flags,ehsize,phsize,phnum,shsize,shnum,shstr=h
    if ident!=b'\x7fELF\x01\x01\x01'+bytes(9) or kind!=2 or machine!=40 or version!=1 or flags!=0x05000200:
        raise ValueError('helper ELF32 little-endian ARM ET_EXEC EABI5 soft-float ABI')
    if ehsize!=52 or phsize!=32 or not 1<=phnum<=16 or phoff<52 or phoff+phnum*32>len(data): raise ValueError('helper ELF program table')
    loads=[]
    for i in range(phnum):
        typ,offset,virtual,physical,filesz,memsz,permissions,alignment=struct.unpack_from('<8I',data,phoff+i*32)
        if typ in (2,3): raise ValueError('helper ELF dynamic/interpreter forbidden')
        if offset+filesz>len(data): raise ValueError('helper ELF file range')
        if typ==1:
            if filesz>memsz or virtual<4096 or virtual+memsz>0x01000000 or memsz>131072 or permissions & ~7 or permissions & 3 == 3:
                raise ValueError('helper ELF load memory/range/permissions')
            if alignment not in (0,1) and (alignment & (alignment-1) or virtual%alignment != offset%alignment): raise ValueError('helper ELF load alignment')
            if any(virtual<end and start<virtual+memsz for start,end,_ in loads): raise ValueError('helper ELF load overlap')
            loads.append((virtual,virtual+memsz,permissions))
        elif typ==0x6474e551 and permissions & 1: raise ValueError('helper executable stack')
    if not any(start<=entry<end and permissions & 1 for start,end,permissions in loads): raise ValueError('helper ELF executable entry')
    if shnum:
        if shnum>128 or shsize!=40 or shoff<52 or shoff+shnum*40>len(data): raise ValueError('helper ELF section table')
        for i in range(shnum):
            _,typ,_,_,offset,size,link,_,_,entsize=struct.unpack_from('<10I',data,shoff+i*40)
            if typ!=8 and offset+size>len(data): raise ValueError('helper ELF section range')
            if typ in (6,11): raise ValueError('helper ELF dynamic section')
            if typ==2:
                if entsize!=16 or size%16 or link>=shnum: raise ValueError('helper ELF symbol table')
                for at in range(offset,offset+size,16):
                    name,_,_,info,_,section=struct.unpack_from('<IIIBBH',data,at)
                    if name and section==0: raise ValueError('helper undefined import')
    return dict(**modules.identity(data),elf='ELF32 little-endian ARM ET_EXEC EABI5 static',entry=entry,load_segments=len(loads),undefined_symbols=0,interpreter=False,dynamic=False)


def build_helper(work,sources,pins):
    work=Path(work).absolute(); base.check_output(work); work.mkdir()
    compiler=tool_identity('arm-none-eabi-gcc')
    if not compiler['version'].splitlines() or compiler['version'].splitlines()[0]!=pins['compiler_version']: raise ValueError('required pinned helper compiler version')
    source=sources['helper.c']; header=phase_header(json.loads(sources['phases.json']))
    (work/'helper.c').write_bytes(source); (work/'phases.h').write_bytes(header)
    maps=[f'-{kind}-prefix-map={work}=/rv1103-basket' for kind in ('ffile','fdebug','fmacro')]
    command=[compiler['path'],'-std=c11','-Os','-Wall','-Wextra','-Werror','-marm','-mcpu=cortex-a7','-mfloat-abi=soft',
             '-nostdlib','-static','-ffreestanding','-fno-builtin','-fno-stack-protector','-fno-unwind-tables','-fno-asynchronous-unwind-tables',
             '-fno-ident',*maps,'-Wl,--build-id=none,-e,_start,-Ttext=0x10000','-o',str(work/'basket-helper'),str(work/'helper.c')]
    environment={'PATH':os.environ.get('PATH','/usr/bin:/bin'),'LC_ALL':'C','TZ':'UTC','SOURCE_DATE_EPOCH':'0'}
    with (work/'build.log').open('wb') as log: modules.run_command(command,work,environment,log,timeout=30,log_limit=65536)
    binary=read_regular(work/'basket-helper',131072); validation=verify_helper_elf(binary)
    return binary,dict(directory=str(work),command=command,command_identity=modules.identity(json.dumps(command).encode()),environment=environment,
                       compiler=compiler,source=modules.identity(source),generated_header=modules.identity(header),output=validation),read_regular(work/'build.log',65536)


def archive_entries(busybox,init,helper,driver_modules):
    if set(driver_modules)!={'i2c-dev.ko','i2c-rk3x.ko'}: raise ValueError('exact two driver modules required')
    entries={name:(stat.S_IFDIR | (0o1777 if name=='tmp' else 0o755),b'',0,0) for name in ('.','bin','proc','sys','tmp','dev','lib','lib/modules','lib/modules/'+modules.RELEASE)}
    entries['bin/busybox']=(stat.S_IFREG|0o755,busybox,0,0)
    entries['bin/basket-helper']=(stat.S_IFREG|0o755,helper,0,0)
    entries['init']=(stat.S_IFREG|0o755,init,0,0)
    for applet in ('sh','mount','uname','cat','head','sleep','awk','insmod','readlink'):
        entries['bin/'+applet]=(stat.S_IFLNK|0o777,b'busybox',0,0)
    for name,content in driver_modules.items(): entries['lib/modules/'+modules.RELEASE+'/'+name]=(stat.S_IFREG|0o644,content,0,0)
    entries['dev/console']=(stat.S_IFCHR|0o600,b'',5,1)
    entries['dev/null']=(stat.S_IFCHR|0o666,b'',1,3)
    return entries


def newc(entries):
    output=bytearray()
    for inode,(name,(mode,body,major,minor)) in enumerate(sorted(entries.items())+[('TRAILER!!!',(0,b'',0,0))],1):
        encoded=name.encode('ascii')+b'\0'; values=[inode,mode,0,0,2 if stat.S_ISDIR(mode) else 1,0,len(body),0,0,major,minor,len(encoded),0]
        output.extend(b'070701'+''.join(f'{v:08x}' for v in values).encode()+encoded)
        output.extend(bytes(-len(output)%4)); output.extend(body); output.extend(bytes(-len(output)%4))
        if len(output)>ARCHIVE_LIMIT: raise ValueError('newc archive byte bound')
    return bytes(output)


def parse_newc(data):
    if len(data)>ARCHIVE_LIMIT: raise ValueError('newc archive byte bound')
    records={}; pos=0; inode=0
    while pos<len(data):
        if pos+110>len(data) or data[pos:pos+6]!=b'070701': raise ValueError('newc header')
        try: fields=[int(data[pos+6+i*8:pos+14+i*8],16) for i in range(13)]
        except ValueError as error: raise ValueError('newc malformed fields') from error
        pos+=110; inode+=1; size=fields[6]; namesize=fields[11]
        if not 1<=namesize<=256 or pos+namesize>len(data): raise ValueError('newc name range')
        namebytes=data[pos:pos+namesize]; pos+=namesize
        if namebytes[-1:]!=b'\0' or b'\0' in namebytes[:-1]: raise ValueError('newc name termination')
        try: name=namebytes[:-1].decode('ascii')
        except UnicodeError as error: raise ValueError('newc name encoding') from error
        padding=-pos%4
        if any(data[pos:pos+padding]): raise ValueError('newc name padding')
        pos+=padding
        if pos+size>len(data): raise ValueError('newc body range')
        body=data[pos:pos+size]; pos+=size; padding=-pos%4
        if pos+padding>len(data) or any(data[pos:pos+padding]): raise ValueError('newc body padding')
        pos+=padding
        mode=fields[1]
        if fields != [inode,mode,0,0,2 if stat.S_ISDIR(mode) else 1,0,size,0,0,fields[9],fields[10],namesize,0]: raise ValueError('newc deterministic metadata')
        if name=='TRAILER!!!':
            if fields[1:]!=[0,0,0,1,0,0,0,0,0,0,11,0] or pos!=len(data): raise ValueError('newc trailer/trailing archive')
            return records
        if name in records: raise ValueError('newc duplicate path')
        if name!='.' and modules.archive_path(name)!=name: raise ValueError('newc canonical path')
        records[name]=(mode,body,fields[9],fields[10])
        if len(records)>64: raise ValueError('newc entry count')
    raise ValueError('newc missing trailer')


def validate_newc(data,expected):
    actual=parse_newc(data)
    if actual!=expected or data!=newc(expected): raise ValueError('newc exact whitelist/mode/body/device mismatch')


def reuse_modules(directory,pins,symvers):
    receiptbytes=checked(Path(directory)/'module-manifest.json',pins['module_receipt'],'reviewed original module receipt')
    receipt=json.loads(receiptbytes)
    if receipt['preparation']!='VERIFIED' or receipt['guest_status']!='NOT_RUN': raise ValueError('module preparation provenance')
    for key,filename in [('script',modules.__file__),('base_script',base.__file__)]:
        if modules.identity(read_regular(filename,SOURCE_LIMIT))!=receipt['preparer'][key]: raise ValueError('module preparer provenance mismatch')
    if receipt['upstream_source']!=modules.SOURCE or receipt['packages']['linux-lts-dev']!=modules.DEV: raise ValueError('module source/package provenance')
    result={}
    for filename,pin in pins['modules'].items():
        data=checked(Path(directory)/filename,pin,filename)
        info=modules.verify_module(data,symvers,'i2c_dev' if filename=='i2c-dev.ko' else 'i2c_rk3x',expected_import_count=48 if filename=='i2c-rk3x.ko' else None)
        if info!=receipt['modules'][filename] or info['signature_present']!=(filename=='i2c-dev.ko'): raise ValueError('module ELF/signature provenance mismatch')
        result[filename]=data
    return result


def verified_uapi(cache,pins):
    apk=base.cached_apk(cache,modules.DEV['url'],modules.DEV['sha256'],modules.DEV['size'],True)
    entries,pkg,stats=modules.read_headers(apk)
    if any(stats.get(k)!=v for k,v in modules.HEADER_STATS.items()): raise ValueError('matching header statistics')
    base.verify_identity(pkg,'linux-lts-dev',modules.DEV['version'])
    for name,pin in pins['uapi'].items():
        if name not in entries or entries[name][0]!='file': raise ValueError('missing pinned UAPI')
        modules.require_identity(entries[name][2],(pin['bytes'],pin['sha256']),name)
    for name,pin in modules.PINNED_HEADERS.items(): modules.require_identity(entries[name][2],pin,name)
    modules.cached_source(cache,True)
    return entries['Module.symvers'][2],entries['.config'][2],dict(package_sha256=modules.DEV['sha256'],files=pins['uapi'],statistics=stats,executed_extracted_helpers=False)

BOOTARGS=base.BOOTARGS

def validate_dtb(data, initrd_end):
    """Exact basket DT output; base ten-node validator remains unchanged."""
    if not 0x03100000 < initrd_end <= 0x04000000: raise ValueError("DTB initrd RAM range")
    if len(data) < 40: raise ValueError('truncated DTB')
    header = struct.unpack_from('>10I', data)
    magic, size, off_s, off_n, off_r, version, compatible, cpu, len_n, len_s = header
    if (magic, size, version, cpu) != (0xd00dfeed, len(data), 17, 0) or compatible > 17:
        raise ValueError('DTB header')
    if not (40 <= off_r <= off_r + 16 <= off_s <= off_s + len_s <= off_n <= off_n + len_n == size):
        raise ValueError('DTB block ranges')
    if off_s % 4 or off_r % 8 or any(data[off_r:off_s]): raise ValueError('DTB reservations/alignment')
    names = data[off_n:off_n + len_n]; end_s = off_s + len_s
    def word(offset):
        if offset + 4 > end_s: raise ValueError('DTB word range')
        return struct.unpack_from('>I', data, offset)[0]
    def string(block, offset):
        if offset >= len(block): raise ValueError('DTB string offset')
        end = block.find(b'\0', offset)
        if end < 0: raise ValueError('DTB missing terminator')
        return block[offset:end].decode('ascii'), end + 1
    nodes = {}; stack = []; pos = off_s
    while pos < end_s:
        token = word(pos); pos += 4
        if token == 1:
            name, end = string(data[:end_s], pos); pos = (end + 3) & ~3
            if '/' in name or (not stack and (name or nodes)): raise ValueError('DTB node name')
            path = (stack[-1].rstrip('/') if stack else '') + '/' + name
            if path in nodes: raise ValueError('duplicate DTB node')
            nodes[path] = {}; stack.append(path)
        elif token == 2:
            if not stack: raise ValueError('DTB unbalanced node')
            stack.pop()
        elif token == 3:
            length, offset = word(pos), word(pos + 4); pos += 8
            name, ignored = string(names, offset)
            if not stack or pos + length > end_s or name in nodes[stack[-1]]: raise ValueError('DTB property range/duplicate')
            nodes[stack[-1]][name] = data[pos:pos+length]; pos = (pos + length + 3) & ~3
        elif token == 4: pass
        elif token == 9:
            if stack or pos != end_s: raise ValueError('DTB trailing structure')
            break
        else: raise ValueError('DTB token')
    else: raise ValueError('missing DTB end')
    def cells(*values): return struct.pack('>' + 'I' * len(values), *values)
    gic = '/interrupt-controller@ff1f1000'; uart = '/serial@ff4c0000'
    required = {
        '/': {'compatible': b'rockchip,rv1103\0', '#address-cells': cells(1), '#size-cells': cells(1)},
        '/aliases': {'serial2': (uart + '\0').encode(), 'serial4': b'/serial@ff4e0000\0', 'i2c3': b'/i2c@ff460000\0'},
        '/chosen': {'stdout-path': b'serial2:115200n8\0', 'bootargs': (BOOTARGS + '\0').encode(), 'linux,initrd-start': cells(0x03100000), 'linux,initrd-end': cells(initrd_end)},
        '/memory@0': {'device_type': b'memory\0', 'reg': cells(0, 0x04000000)},
        '/cpus': {'#address-cells': cells(1), '#size-cells': cells(0)},
        '/cpus/cpu@0': {'device_type': b'cpu\0', 'compatible': b'arm,cortex-a7\0', 'reg': cells(0)},
        gic: {'compatible': b'arm,gic-400\0', '#interrupt-cells': cells(3), 'interrupt-controller': b'', 'reg': cells(0xff1f1000,0x1000,0xff1f2000,0x2000)},
        '/timer': {'compatible': b'arm,armv7-timer\0', 'interrupts': cells(1,13,0x104,1,14,0x104), 'clock-frequency': cells(24000000)},
        '/clock24m': {'compatible': b'fixed-clock\0', '#clock-cells': cells(0), 'clock-frequency': cells(24000000)},
        uart: {'compatible': b'snps,dw-apb-uart\0', 'reg': cells(0xff4c0000,0x100), 'interrupts': cells(0,27,4), 'reg-shift': cells(2), 'reg-io-width': cells(4), 'clock-frequency': cells(24000000), 'clock-names': b'baudclk\0apb_pclk\0', 'status': b'okay\0'}
    }
    required['/i2c@ff460000'] = {'compatible': b'rockchip,rv1106-i2c\0rockchip,rk3399-i2c\0', 'reg': cells(0xff460000,0x1000), 'interrupts': cells(0,21,4), '#address-cells': cells(1), '#size-cells': cells(0), 'clock-names': b'i2c\0pclk\0', 'clock-frequency': cells(100000), 'status': b'okay\0'}
    required['/serial@ff4e0000'] = dict(required[uart], reg=cells(0xff4e0000,0x100), interrupts=cells(0,29,4))
    if set(nodes) != set(required): raise ValueError('DTB unsupported nodes')
    extras = {'/': {'model', 'interrupt-parent'}, gic: {'#address-cells', 'phandle'}, '/clock24m': {'phandle'}, uart: {'clocks'}, '/serial@ff4e0000': {'clocks'}, '/i2c@ff460000': {'clocks'}}
    for path, props in nodes.items():
        if set(props) - set(required[path]) - extras.get(path, set()): raise ValueError('DTB unsupported properties')

    for path, props in required.items():
        for key, value in props.items():
            if nodes[path].get(key) != value: raise ValueError(f'DTB geometry mismatch {path}:{key}')
    if nodes['/'].get('model') != b'LabWired Luckfox Pico Mini A reduced direct boot\0' or nodes[gic].get('#address-cells') != cells(0): raise ValueError('DTB profile properties')
    clock = nodes['/clock24m'].get('phandle'); interrupt = nodes[gic].get('phandle')
    if not clock or not interrupt or len(clock) != 4 or len(interrupt) != 4 or clock == interrupt or clock in (bytes(4), b'\xff' * 4) or interrupt in (bytes(4), b'\xff' * 4):
        raise ValueError('DTB invalid phandle')
    if nodes['/'].get('interrupt-parent') != interrupt or any(nodes[path].get('clocks') != clock * 2 for path in (uart, '/serial@ff4e0000', '/i2c@ff460000')):
        raise ValueError('DTB reference mismatch')

def prepare(output,cache,base_directory,module_directory):
    output=Path(output).absolute()
    base.check_output(output)
    cache=Path(cache)
    if cache.is_symlink() or any(p.is_symlink() for p in cache.parents): raise ValueError('symlink APK cache')
    output=Path(output); output.parent.mkdir(parents=True,exist_ok=True)
    modules.disk_guard(output.parent)
    pins=load_pins(); sources=fixture_sources(pins)
    symvers,devconfig,uapi=verified_uapi(cache,pins)
    original={name:checked(Path(base_directory)/name,pin,'base '+name) for name,pin in pins['base'].items()}
    base.verify_config(original['kernel.config']); modules.verify_config_pair(original['kernel.config'],devconfig)
    for option in ('CONFIG_SERIAL_8250_NR_UARTS=8','CONFIG_SERIAL_8250_RUNTIME_UARTS=8','CONFIG_I2C=y','CONFIG_I2C_CHARDEV=m'):
        if option not in original['kernel.config'].decode('ascii').splitlines(): raise ValueError('missing basket kernel option '+option)
    kernel=original['zImage']
    if struct.unpack_from('<III',kernel,0x24)!=(0x016f2818,0,len(kernel)): raise ValueError('base zImage header')
    base.validate_dtb(original['rv1103-direct.dtb'],0x03100000+len(original['initramfs.cpio']))
    base_records=parse_newc(original['initramfs.cpio']); busybox=base_records['bin/busybox'][1]
    if base.initramfs(busybox,original['init'])!=original['initramfs.cpio']: raise ValueError('strict original base archive')
    modules.require_identity(busybox,base.PACKAGES['busybox-static']['members']['bin/busybox.static'],'base busybox')
    drivers=reuse_modules(module_directory,pins,symvers)
    dtc=tool_identity('dtc')
    with tempfile.TemporaryDirectory(prefix='.rv1103-basket-',dir=output.parent) as tmp:
        work=Path(tmp); builds=[]; binaries=[]; archives=[]; trees=[]; logs={}
        for label in ('build-a','build-b'):
            binary,build,log=build_helper(work/label,sources,pins)
            modules.require_identity(binary,(pins['helper']['bytes'],pins['helper']['sha256']),'reviewed helper')
            entries=archive_entries(busybox,sources['init'],binary,drivers); archive=newc(entries); validate_newc(archive,entries)
            end=0x03100000+len(archive)
            if end>0x04000000: raise ValueError('basket initrd exceeds64M RAM')
            directory=work/label; dts=sources['rv1103-basket.dts'].replace(b'@INITRD_END@',f'0x{end:08x}'.encode())
            (directory/'rv1103-basket.dts').write_bytes(dts); (directory/'initramfs.cpio').write_bytes(archive)
            command=[dtc['path'],'-I','dts','-O','dtb','-o',str(directory/'rv1103-basket.dtb'),str(directory/'rv1103-basket.dts')]
            with (directory/'dtc.log').open('wb') as dtlog: modules.run_command(command,directory,build['environment'],dtlog,timeout=10,log_limit=65536)
            tree=read_regular(directory/'rv1103-basket.dtb',131072); validate_dtb(tree,end)
            for name,data in [('initramfs.cpio',archive),('rv1103-basket.dtb',tree)]:
                expected=pins['outputs'][name]; modules.require_identity(data,(expected['bytes'],expected['sha256']),'reviewed '+name)
            if 0x03000000+len(tree)>0x03100000 or 0x02000000+len(kernel)>0x03000000: raise ValueError('loaded artifact reserved range overlap')
            binaries.append(binary); archives.append(archive); trees.append(tree)
            builds.append(dict(**build,dtc_command=command,dtc=dtc,dtb=modules.identity(tree),initramfs=modules.identity(archive)))
            logs[label+'.log']=log; logs[label+'-dtc.log']=read_regular(directory/'dtc.log',65536)
        if binaries[0]!=binaries[1] or archives[0]!=archives[1] or trees[0]!=trees[1]: raise ValueError('independent build helper/DT/archive bytes differ')
        files={'zImage':kernel,'kernel.config':original['kernel.config'],'init':sources['init'],'basket-helper':binaries[0],
               'initramfs.cpio':archives[0],'rv1103-basket.dts':dts,'rv1103-basket.dtb':trees[0],'phases.json':sources['phases.json'],**drivers,**logs}
        receipt=dict(schema=1,profile=pins['profile'],preparation='VERIFIED',guest_status='NOT_RUN',physical_acceptance='UNAVAILABLE',
                     pins=modules.identity(read_regular(FIXTURE/'pins.json',SOURCE_LIMIT)),preparer=modules.identity(read_regular(__file__,SOURCE_LIMIT)),
                     sources=pins['sources'],uapi=uapi,module_provenance=pins['module_receipt'],modules=pins['modules'],builds=builds,
                     reproducibility=dict(independent_builds=2,helper_bytes_equal=True,dtb_bytes_equal=True,archive_bytes_equal=True),
                     artifacts={name:dict(**modules.identity(content),address={'zImage':0x02000000,'rv1103-basket.dtb':0x03000000,'initramfs.cpio':0x03100000}.get(name)) for name,content in files.items()},
                     ram_base=0,ram_bytes=0x04000000,initrd_end=end,decompression_start=0x8000,decompression_end=0x016e5850,
                     timer_hz=24000000,bootargs=BOOTARGS,archive_whitelist=sorted(entries),phase_contract=json.loads(sources['phases.json']),
                     guest_load_policy='ordinary Busybox insmod; no signature/CRC/version bypass; real exit status and taint required',
                     limitations=['preparation and static ELF/UAPI checks only; guest execution NOT_RUN',
                                  'functional24MHz window; no hardware UART/RF or silicon power-threshold claim',
                                  'carrier disconnect and full plant cut/retirement/fresh fixture recovery are future host gates'])
        stage=work/'published'; stage.mkdir()
        for name,content in files.items(): (stage/name).write_bytes(content)
        (stage/'artifact-manifest.json').write_text(json.dumps(receipt,indent=2,sort_keys=True)+'\n')
        base.check_output(output); os.rename(stage,output)
    return receipt

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--cache',type=Path,required=True,help='verified offline APK/source cache')
    parser.add_argument('--base-artifacts',type=Path,required=True)
    parser.add_argument('--module-artifacts',type=Path,required=True)
    args=parser.parse_args()
    try:
        receipt=prepare(args.output,args.cache,args.base_artifacts,args.module_artifacts)
        print(json.dumps(dict(preparation=receipt['preparation'],guest_status=receipt['guest_status'],output=str(args.output))))
    except (ValueError,OSError,UnicodeError,KeyError,RecursionError) as error:
        parser.exit(1,f'basket preparation failed: {error}\n')
