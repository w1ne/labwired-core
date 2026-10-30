#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
"""Regenerate and check `PIC18_DEVICES` in crates/core/src/cpu/pic18.rs.

Reads part names (`pic18f4520`, one per line) on stdin. For each part it
checks that every SFR address and bit position the PIC18 model implements
matches the SDCC 4.4.0 device header, and reports flash size (gputils
`<part>_g.lkr` code page) and Access Bank split (SDCC `pic16devices.txt`).

    SDCC_SHARE=/opt/sdcc-4.4.0/share/sdcc GPUTILS_LKR=/usr/share/gputils/lkr \
        python3 scripts/pic18/check_sfr_map.py --table < parts.txt

With --check-table it re-verifies every row already in pic18.rs.
Without either flag it prints JSON with the mismatches per part. The SDCC
headers are not vendored; point SDCC_SHARE at an upstream SDCC release
(distribution packages often omit the pic16 port).
"""
import re, sys, json, os
SDCC = os.environ.get('SDCC_SHARE', os.path.expanduser('~/tools/sdcc-up/sdcc-4.4.0/share/sdcc'))
HDR = SDCC + '/non-free/include/pic16/pic%s.h'
DEV = SDCC + '/include/pic16/pic16devices.txt'
LKR = os.environ.get('GPUTILS_LKR', '/usr/share/gputils/lkr') + '/%s_g.lkr'
SFRS = {'WREG':0xFE8,'STATUS':0xFD8,'BSR':0xFE0,'FSR0L':0xFE9,'FSR0H':0xFEA,'FSR1L':0xFE1,'FSR1H':0xFE2,
 'FSR2L':0xFD9,'FSR2H':0xFDA,'PLUSW2':0xFDB,'PREINC2':0xFDC,'POSTDEC2':0xFDD,'POSTINC2':0xFDE,'INDF2':0xFDF,
 'PLUSW1':0xFE3,'PREINC1':0xFE4,'POSTDEC1':0xFE5,'POSTINC1':0xFE6,'INDF1':0xFE7,'PLUSW0':0xFEB,'PREINC0':0xFEC,
 'POSTDEC0':0xFED,'POSTINC0':0xFEE,'INDF0':0xFEF,'PRODL':0xFF3,'PRODH':0xFF4,'TABLAT':0xFF5,'TBLPTRL':0xFF6,
 'TBLPTRH':0xFF7,'TBLPTRU':0xFF8,'PCL':0xFF9,'PCLATH':0xFFA,'PCLATU':0xFFB,'STKPTR':0xFFC,'TOSL':0xFFD,'TOSH':0xFFE,
 'TOSU':0xFFF,'INTCON':0xFF2,'INTCON2':0xFF1,'INTCON3':0xFF0,'PIR1':0xF9E,'PIR2':0xFA1,'PIE1':0xF9D,'PIE2':0xFA0,
 'IPR1':0xF9F,'IPR2':0xFA2,'RCON':0xFD0,'TXREG':0xFAD,'TXSTA':0xFAC,'RCSTA':0xFAB,'SPBRG':0xFAF,
 'PORTA':0xF80,'PORTB':0xF81,'PORTC':0xF82,'LATA':0xF89,'LATB':0xF8A,'LATC':0xF8B,'TRISA':0xF92,'TRISB':0xF93,'TRISC':0xF94}
ALIASES = {'TXREG':['TXREG','TXREG1'],'TXSTA':['TXSTA','TXSTA1'],'RCSTA':['RCSTA','RCSTA1'],'SPBRG':['SPBRG','SPBRG1'],
 'INTCON':['INTCON','INTCON1']}
BITS = {'INTCON':{'GIE':7,'PEIE':6,'TMR0IE':5,'INT0IE':4,'RBIE':3,'TMR0IF':2,'INT0IF':1,'RBIF':0},
 'INTCON2':{'TMR0IP':2,'RBIP':0},'INTCON3':{'INT2IP':7,'INT1IP':6,'INT2IE':4,'INT1IE':3,'INT2IF':1,'INT1IF':0},
 'RCON':{'IPEN':7,'TO':3,'PD':2},'TXSTA':{'TXEN':5,'TRMT':1},'RCSTA':{'SPEN':7},'PIR1':{'TXIF':4},
 'STKPTR':{'STKFUL':7,'STKUNF':6}}
BIT_ALIASES = {'GIE':['GIE','GIEH'],'PEIE':['PEIE','GIEL'],'TMR0IE':['TMR0IE','T0IE'],'INT0IE':['INT0IE','INT0E'],
 'RBIE':['RBIE','IOCIE'],'TMR0IF':['TMR0IF','T0IF'],'INT0IF':['INT0IF','INT0F'],'RBIF':['RBIF','IOCIF'],
 'INT2IE':['INT2IE','INT2E'],'INT1IE':['INT1IE','INT1E'],'INT2IF':['INT2IF','INT2F'],'INT1IF':['INT1IF','INT1F'],
 'TO':['TO','NOT_TO','nTO'],'PD':['PD','NOT_PD','nPD'],'TXIF':['TXIF','TX1IF'],'TMR0IP':['TMR0IP','T0IP'],
 'RBIP':['RBIP','IOCIP'],'STKFUL':['STKFUL','STKOVF'],'TXEN':['TXEN','TXEN1'],'TRMT':['TRMT','TRMT1'],'SPEN':['SPEN','SPEN1'],
 'TXIF':['TXIF','TX1IF']}

def devices():
    out, cur, raw = {}, None, {}
    for line in open(DEV):
        line=line.split('#')[0].strip()
        if not line: continue
        k,*v=line.split()
        if k=='name': cur=v[0]; raw[cur]={}
        elif k=='using': raw[cur].update({kk:vv for kk,vv in raw[v[0]].items()})
        elif k in('ramsize','split'): raw[cur][k]=int(v[0],0)
    return raw

def flash_of(part):
    p=LKR%part
    if not os.path.exists(p): return None
    for l in open(p):
        m=re.match(r'\s*CODEPAGE\s+NAME=page\s+START=0x0+\s+END=(0x[0-9A-Fa-f]+)',l)
        if m: return int(m.group(1),16)+1
    return None

def parse_hdr(part):
    p=HDR%part
    if not os.path.exists(p): return None,None
    t=open(p).read()
    addr={}
    for m in re.finditer(r'extern __at\((0x[0-9A-Fa-f]+)\) __sfr (\w+);',t): addr[m.group(2)]=int(m.group(1),16)
    bits={}
    for m in re.finditer(r'typedef (union|struct)\s*\{(.*?)\}\s*__(\w+)bits_t;',t,re.S):
        reg=m.group(3); body=m.group(2); d={}
        structs=[body] if m.group(1)=='struct' else [x.group(1) for x in re.finditer(r'struct\s*\{(.*?)\}',body,re.S)]
        for s in structs:
            pos=0
            for f in re.finditer(r'unsigned\s*(\w*)\s*:\s*(\d+)\s*;',s):
                n,w=f.group(1),int(f.group(2))
                if n and w==1: d.setdefault(n,pos)
                pos+=w
        bits[reg]=d
    return addr,bits

def check(part):
    addr,bits=parse_hdr(part)
    if addr is None: return ['no SDCC header']
    errs=[]
    for name,a in SFRS.items():
        cands=ALIASES.get(name,[name])
        got=[addr[c] for c in cands if c in addr]
        if not got: errs.append(f'{name} missing')
        elif a not in got: errs.append(f'{name}@{got[0]:#x}')
    for reg,bm in BITS.items():
        rb=None
        for c in ALIASES.get(reg,[reg]):
            if c in bits: rb=bits[c]; break
        if rb is None: errs.append(f'{reg}bits missing'); continue
        for b,pos in bm.items():
            got=[rb[c] for c in BIT_ALIASES.get(b,[b]) if c in rb]
            if not got: errs.append(f'{reg}.{b} missing')
            elif pos not in got: errs.append(f'{reg}.{b}@{got[0]}')
    return errs

if __name__=='__main__':
    devs=devices()
    if '--check-table' in sys.argv:
        rs=open(os.path.join(os.path.dirname(__file__),'../../crates/core/src/cpu/pic18.rs')).read()
        rows=re.findall(r'^    \("(pic18\w+)", (\d+), 0x([0-9A-F]+)\),$',rs,re.M)
        bad=0
        for name,flash,split in rows:
            k=name.replace('pic','',1); d=devs.get(k,{}); e=check(k)
            if e or flash_of(k)!=int(flash) or d.get('split')!=int(split,16):
                bad+=1; print(name,'MISMATCH',e,flash_of(k),d.get('split'))
        print(f'{len(rows)} rows checked, {bad} mismatches'); sys.exit(1 if bad or not rows else 0)
    parts=[l.strip() for l in sys.stdin if l.strip()]
    res={}
    for p in parts:
        k=p.replace('pic','',1)
        d=devs.get(k)
        e=check(k)
        res[p]={'split':d.get('split') if d else None,'ram':d.get('ramsize') if d else None,'flash':flash_of(k),'errs':e}
    if '--table' in sys.argv:
        ok=sorted((k for k,v in res.items() if not v['errs']), key=lambda k:(k.replace('lf','f'),k))
        for k in ok:
            print(f'    ("{k}", {res[k]["flash"]}, 0x{res[k]["split"]:X}),')
    else:
        json.dump(res,sys.stdout,indent=1)
