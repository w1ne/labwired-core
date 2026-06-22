TARGET := f103_uds_ecu
BUILD_DIR := build
UDSLIB_DIR ?= $(HOME)/projects/udslib
CROSS ?= arm-none-eabi-
CC := $(CROSS)gcc

CPUFLAGS := -mcpu=cortex-m3 -mthumb
COMMON_CFLAGS := $(CPUFLAGS) -ffreestanding -fno-builtin -ffunction-sections \
                 -fdata-sections -Os -g -I$(UDSLIB_DIR)/include -I$(UDSLIB_DIR)/src/core
APP_CFLAGS := $(COMMON_CFLAGS) -Wall -Wextra -Werror
UDS_CFLAGS := $(COMMON_CFLAGS) -Wall -Wextra -Wno-unused-parameter \
              -Wno-missing-field-initializers
LDFLAGS := -nostdlib -T minimal.ld -Wl,--gc-sections -Wl,-Map=$(BUILD_DIR)/$(TARGET).map

APP_SRCS := startup.c main.c
UDS_SRCS := \
  $(UDSLIB_DIR)/src/core/uds_core.c \
  $(UDSLIB_DIR)/src/services/uds_service_data.c \
  $(UDSLIB_DIR)/src/services/uds_service_flash.c \
  $(UDSLIB_DIR)/src/services/uds_service_io.c \
  $(UDSLIB_DIR)/src/services/uds_service_link.c \
  $(UDSLIB_DIR)/src/services/uds_service_maintenance.c \
  $(UDSLIB_DIR)/src/services/uds_service_mem.c \
  $(UDSLIB_DIR)/src/services/uds_service_roe.c \
  $(UDSLIB_DIR)/src/services/uds_service_security.c \
  $(UDSLIB_DIR)/src/services/uds_service_session.c \
  $(UDSLIB_DIR)/src/services/uds_dtc_store.c \
  $(UDSLIB_DIR)/src/transport/uds_tp_isotp.c

APP_OBJS := $(patsubst %.c,$(BUILD_DIR)/%.o,$(APP_SRCS))
UDS_OBJS := $(patsubst $(UDSLIB_DIR)/%.c,$(BUILD_DIR)/udslib/%.o,$(UDS_SRCS))
ELF := $(BUILD_DIR)/$(TARGET).elf
# Same firmware compiled with -DBROKEN_NCR to reproduce udslib issue #29
# (N_Cr clock-source mismatch drops the consecutive frame).
BROKEN_ELF := $(BUILD_DIR)/$(TARGET)_broken.elf

all: check-udslib $(ELF) $(BROKEN_ELF)

$(BUILD_DIR)/main_broken.o: main.c | $(BUILD_DIR)
	$(CC) $(APP_CFLAGS) -DBROKEN_NCR -c $< -o $@

$(BROKEN_ELF): $(BUILD_DIR)/startup.o $(BUILD_DIR)/main_broken.o $(UDS_OBJS) minimal.ld
	$(CC) $(CPUFLAGS) -nostdlib -T minimal.ld -Wl,--gc-sections \
	  $(BUILD_DIR)/startup.o $(BUILD_DIR)/main_broken.o $(UDS_OBJS) -o $@

check-udslib:
	@test -f "$(UDSLIB_DIR)/include/uds/uds_core.h" || \
	  (echo "UDSLIB_DIR must point at a UDSLib checkout" >&2; exit 1)

$(ELF): $(APP_OBJS) $(UDS_OBJS) minimal.ld
	$(CC) $(CPUFLAGS) $(LDFLAGS) $(APP_OBJS) $(UDS_OBJS) -o $@

$(BUILD_DIR)/%.o: %.c | $(BUILD_DIR)
	$(CC) $(APP_CFLAGS) -c $< -o $@

$(BUILD_DIR)/udslib/%.o: $(UDSLIB_DIR)/%.c | $(BUILD_DIR)
	@mkdir -p $(dir $@)
	$(CC) $(UDS_CFLAGS) -c $< -o $@

$(BUILD_DIR):
	mkdir -p $@

clean:
	rm -rf $(BUILD_DIR)

.PHONY: all check-udslib clean
