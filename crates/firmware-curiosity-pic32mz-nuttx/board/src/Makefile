############################################################################
# curiosity-pic32mz-ef/src/Makefile
#
# SPDX-License-Identifier: Apache-2.0
#
# Derived from the Apache NuttX 12.11.0 flipnclick-pic32mz board.
############################################################################

include $(TOPDIR)/Make.defs

CSRCS = pic32mz_boot.c pic32mz_bringup.c pic32mz_userleds.c

ifeq ($(CONFIG_BOARDCTL),y)
CSRCS += pic32mz_appinit.c
endif

include $(TOPDIR)/boards/Board.mk
