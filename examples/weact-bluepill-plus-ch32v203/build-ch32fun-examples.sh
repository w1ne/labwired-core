#!/usr/bin/env bash
# Build unmodified ch32fun (cnlohr/ch32fun, MIT) CH32V20x examples for
# the CH32V203C8T6 into examples/weact-bluepill-plus-ch32v203/ch32fun/:
#
#   blink.elf            examples_v20x/blink: PA0, PA2, PA4, PA7 toggled every
#                        Delay_Ms(250) at the 144 MHz SystemInit leaves (HSI
#                        x18 PLL), timed by the QingKe V4 64-bit SysTick.
#   exti_blink.elf       examples_v20x/exti_blink: EXTI line 1 on PA1 (rising
#                        edge, AFIO_EXTICR1 = PA) through PFIC interrupt 23
#                        (EXTI1_IRQn); the handler toggles PA10.
#   debugprintfdemo.elf  examples_v20x/debugprintfdemo: "+N" / "-N[c]" lines
#                        through the WCH-LinkE debug printf channel (DMDATA0/1
#                        at 0xE0000380) every Delay_Ms(100), PD0/PC0 toggling.
#   spidmatest.elf       examples_v20x/spidmatest: SPI2 TX fed by circular
#                        DMA1 channel 5, refilled from the HT/TC interrupt.
#   canbus_network.elf   examples_v20x/canbus_network as committed upstream
#                        (TRANSMIT 0): CAN1 on PA11/PA12 at 25 kbit/s, filter 0
#                        accepts all into FIFO0, polled every Delay_Ms(50),
#                        each frame printed on the debug printf channel.
#   canbus_network_tx.elf the same source with `#define TRANSMIT 1`, the one
#                        edit its README says to make on the second node ("Flash
#                        the code with TRANSMIT 1 on one device and TRANSMIT 0
#                        on the other"): extended ID 0xBADC0DE, 8 bytes,
#                        every Delay_Ms(1000).
#
# The compile and link lines are the ones ch32fun.mk emits for
# TARGET_MCU=CH32V203 with TARGET_MCU_PACKAGE=CH32V203C8T6 (rv32imac/ilp32,
# -Os -flto, MCU_PACKAGE=1, TARGET_MCU_LD=2: 64 KB flash, 20 KB RAM), with the
# toolchain prefix made explicit. The examples' own Makefiles name the
# CH32V203G6U6; the C8T6 is the part on the WeAct BluePill-Plus-CH32, and the
# sources do not depend on the package. ch32fun is MIT licensed, so the ELFs
# are committed; LICENSE-ch32fun carries the notice.
#
# Toolchain: riscv64-unknown-elf-gcc 13.2 (Ubuntu gcc-riscv64-unknown-elf), as
# for examples/ch32v003-evt/build-ch32fun-examples.sh:
#   RISCV_PREFIX=~/tools/riscv64-deb/root/usr \
#   NEWLIB=~/tools/riscv64-deb/root/usr/lib/picolibc/riscv64-unknown-elf/include
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out_dir="${repo_root}/examples/weact-bluepill-plus-ch32v203/ch32fun"
mkdir -p "${out_dir}"

ch32fun_commit="b28d285460ce052a179dba8f4a22c6f73ff8e328"
src="${CH32FUN_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/labwired/ch32fun-${ch32fun_commit}}"
prefix="${RISCV_PREFIX:-/usr}/bin/riscv64-unknown-elf-"
newlib="${NEWLIB:-/usr/include/newlib}"

if [[ ! -f "${src}/ch32fun/ch32fun.c" ]]; then
  rm -rf "${src}"
  git init -q "${src}"
  git -C "${src}" fetch -q --depth 1 https://github.com/cnlohr/ch32fun.git "${ch32fun_commit}"
  git -C "${src}" checkout -q FETCH_HEAD
fi
cp "${src}/LICENSE" "${out_dir}/LICENSE-ch32fun"

ld="${out_dir}/generated_ch32v203.ld"
"${prefix}gcc" -E -P -x c -DTARGET_MCU=CH32V203 -DMCU_PACKAGE=1 -DTARGET_MCU_LD=2 \
  -DTARGET_MCU_MEMORY_SPLIT= "${src}/ch32fun/ch32fun.ld" > "${ld}"

# canbus_network_tx: the README's TRANSMIT 1 node, from a copy of the example
# directory (funconfig.h comes along) with that single define flipped.
tx_dir="$(mktemp -d)"
trap 'rm -rf "${tx_dir}"' EXIT
mkdir -p "${tx_dir}/canbus_network_tx"
cp "${src}/examples_v20x/canbus_network/funconfig.h" "${tx_dir}/canbus_network_tx/"
sed 's/^#define TRANSMIT 0 /#define TRANSMIT 1 /' \
  "${src}/examples_v20x/canbus_network/canbus_network.c" \
  > "${tx_dir}/canbus_network_tx/canbus_network_tx.c"
grep -q '^#define TRANSMIT 1 ' "${tx_dir}/canbus_network_tx/canbus_network_tx.c"

for example in blink exti_blink debugprintfdemo spidmatest canbus_network canbus_network_tx; do
  example_dir="${src}/examples_v20x/${example}"
  [[ "${example}" == canbus_network_tx ]] && example_dir="${tx_dir}/${example}"
  "${prefix}gcc" -o "${out_dir}/${example}.elf" \
    "${src}/ch32fun/ch32fun.c" "${example_dir}/${example}.c" \
    -g -Os -flto -ffunction-sections -fdata-sections -fmessage-length=0 \
    -msmall-data-limit=8 -fno-tree-loop-distribute-patterns \
    -march=rv32imac -mabi=ilp32 -DCH32V20x=1 -static-libgcc \
    -I"${newlib}" -I"${src}/extralibs" -I"${src}/ch32fun" -nostdlib \
    -I"${example_dir}" -Wall \
    -L"${src}/misc" -lgcc -T "${ld}" -Wl,--gc-sections -lgcc \
    -ffile-prefix-map="${src}"=ch32fun
  "${prefix}strip" --strip-debug "${out_dir}/${example}.elf"
  chmod 0644 "${out_dir}/${example}.elf"
  "${prefix}size" "${out_dir}/${example}.elf"
done
rm -f "${ld}"
