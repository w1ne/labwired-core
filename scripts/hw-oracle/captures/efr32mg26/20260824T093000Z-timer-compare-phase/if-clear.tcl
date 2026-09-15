init
reset halt
mww 0x40008064 0x04004630
mww 0x20000000 0xE7FEE7FE
reg pc 0x20000001
reg msp 0x20080000
reg primask 1
proc rd {a} { return [lindex [read_memory $a 32 1] 0] }
mww 0x40048030 0
mww 0x40048004 0x0FFC0000
mww 0x40048060 0x00000002
mww 0x40048030 1
mww 0x4004801C 0x0000FFFF
mww 0x40048068 0x00008000
mww 0x40048024 0x00007FF8
mww 0x4004800C 1
resume
halt
echo "FIRED    cnt=[format 0x%04x [rd 0x40048024]] if=[format 0x%08x [rd 0x40048014]]"
mww 0x40048014 0xFFFFFFFF
echo "AFTER_IF_W1C    if=[format 0x%08x [rd 0x40048014]]  (frozen, nothing can re-set it)"
mww 0x4004A014 0xFFFFFFFF
echo "AFTER_CLR_ALIAS if=[format 0x%08x [rd 0x40048014]]"
exit
