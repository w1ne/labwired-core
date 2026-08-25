init
reset halt
mww 0x40008064 0x04004630
mww 0x20000000 0xE7FEE7FE
reg pc 0x20000001
reg msp 0x20080000
reg primask 1
proc rd {a} { return [lindex [read_memory $a 32 1] 0] }
proc setup {presc top oc} {
  mww 0x40048030 0
  mww 0x40048004 [expr {$presc << 18}]
  mww 0x40048060 0x00000002
  mww 0x40048030 1
  mww 0x4004801C $top
  mww 0x40048068 $oc
  mww 0x4004A014 0xFFFFFFFF
  mww 0x4004800C 1
  echo "SETUP presc=$presc top=[format 0x%04x [rd 0x4004801C]] oc=[format 0x%04x [rd 0x40048068]] cc0cfg=[format 0x%08x [rd 0x40048060]] status=[rd 0x40048010]"
}
proc sweep {tag lo hi reps} {
  for {set s $lo} {$s <= $hi} {incr s} {
    for {set r 0} {$r < $reps} {incr r} {
      mww 0x40048024 $s
      mww 0x4004A014 0xFFFFFFFF
      set pre [rd 0x40048014]
      resume
      halt
      echo "$tag start=[format 0x%04x $s] ifpre=$pre landed=[format 0x%04x [rd 0x40048024]] if=[format 0x%08x [rd 0x40048014]]"
    }
  }
}
# A: a different OC entirely, to prove the boundary tracks OC and is not an artifact of 0x8000
setup 1023 0x0000FFFF 0x00001234
sweep A 0x1220 0x1240 6
# B: a small TOP, so the counter wraps often
setup 1023 0x000000FF 0x00000080
sweep B 0x0070 0x0090 6
# C: OC == TOP — does the flag appear as the counter wraps to 0?
setup 1023 0x000000FF 0x000000FF
sweep C 0x00F0 0x00FF 8
# D: OC ABOVE TOP — a value the counter can never hold. Does it EVER latch?
setup 1023 0x000000FF 0x00000180
mww 0x40048024 0x00000000
mww 0x4004A014 0xFFFFFFFF
for {set i 0} {$i < 40} {incr i} { resume; halt }
echo "D_ABOVE_TOP after 40 resumes: cnt=[format 0x%04x [rd 0x40048024]] if=[format 0x%08x [rd 0x40048014]]"
# E: control — same run, OC back inside range, proves the rig still latches
setup 1023 0x000000FF 0x00000080
mww 0x40048024 0x00000000
mww 0x4004A014 0xFFFFFFFF
for {set i 0} {$i < 40} {incr i} { resume; halt }
echo "E_CONTROL   after 40 resumes: cnt=[format 0x%04x [rd 0x40048024]] if=[format 0x%08x [rd 0x40048014]]"
exit
