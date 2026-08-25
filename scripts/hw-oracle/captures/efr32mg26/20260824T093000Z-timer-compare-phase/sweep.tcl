init
reset halt
mww 0x40008064 0x04004630
mww 0x20000000 0xE7FEE7FE
reg pc 0x20000001
reg msp 0x20080000
reg primask 1
proc rd {a} { return [lindex [read_memory $a 32 1] 0] }
# EN=0 for the CONFIG registers: CFG and CC0_CFG both need it.
mww 0x40048030 0
mww 0x40048004 0x0FFC0000
mww 0x40048060 0x00000002
# EN=1 for the RUNTIME registers: TOP, CC0_OC, CNT.
mww 0x40048030 1
mww 0x4004801C 0x0000FFFF
mww 0x40048068 0x00008000
mww 0x40048024 0x00007F00
mww 0x4004A014 0xFFFFFFFF
echo "READBACK cfg=[format 0x%08x [rd 0x40048004]] cc0cfg=[format 0x%08x [rd 0x40048060]] top=[format 0x%04x [rd 0x4004801C]] cc0oc=[format 0x%04x [rd 0x40048068]] en=[rd 0x40048030]"
mww 0x4004800C 1
echo "AFTER_START status=[format 0x%08x [rd 0x40048010]] if=[format 0x%08x [rd 0x40048014]]"
for {set s 0x7FE0} {$s <= 0x8000} {incr s} {
  for {set r 0} {$r < 8} {incr r} {
    mww 0x40048024 $s
    mww 0x4004A014 0xFFFFFFFF
    set ifpre [rd 0x40048014]
    resume
    halt
    set post [rd 0x40048024]
    set ifpost [rd 0x40048014]
    echo "D start=[format 0x%04x $s] ifpre=$ifpre landed=[format 0x%04x $post] if=[format 0x%08x $ifpost]"
  }
}
exit
