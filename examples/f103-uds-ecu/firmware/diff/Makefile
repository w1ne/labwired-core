TARGET := f103_uds_diff
BUILD_DIR := build
# Pinned UDSLib version (see ../Makefile). Fetch with `make -C .. fetch-udslib`.
UDSLIB_VERSION ?= 2.0.0
UDSLIB_DIR ?= $(HOME)/projects/udslib
CROSS ?= arm-none-eabi-
CC := $(CROSS)gcc

CPUFLAGS := -mcpu=cortex-m3 -mthumb
COMMON_CFLAGS := $(CPUFLAGS) -ffreestanding -fno-builtin -ffunction-sections \
                 -fdata-sections -Os -g -I$(UDSLIB_DIR)/include -I$(UDSLIB_DIR)/src/core
APP_CFLAGS := $(COMMON_CFLAGS) -Wall -Wextra -Werror
UDS_CFLAGS := $(COMMON_CFLAGS) -Wall -Wextra -Wno-unused-parameter \
              -Wno-missing-field-initializers
LDFLAGS := -nostdlib -T diff.ld -Wl,--gc-sections -Wl,-Map=$(BUILD_DIR)/$(TARGET).map

# startup.c and minimal.ld are reused from the parent firmware directory.
APP_OBJS := $(BUILD_DIR)/startup.o $(BUILD_DIR)/main.o
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
  $(UDSLIB_DIR)/src/transport/uds_tp_isotp.c

UDS_OBJS := $(patsubst $(UDSLIB_DIR)/%.c,$(BUILD_DIR)/udslib/%.o,$(UDS_SRCS))
ELF := $(BUILD_DIR)/$(TARGET).elf

all: check-udslib $(ELF)

check-udslib:
	@test -f "$(UDSLIB_DIR)/include/uds/uds_core.h" || \
	  (echo "UDSLIB_DIR must point at a UDSLib checkout (try: make -C .. fetch-udslib)" >&2; exit 1)
	@grep -q 'UDS_VERSION_STR "$(UDSLIB_VERSION)"' "$(UDSLIB_DIR)/include/uds/uds_version.h" || \
	  (echo "UDSLIB_DIR is not UDSLib v$(UDSLIB_VERSION); run 'make -C .. fetch-udslib' or repoint UDSLIB_DIR" >&2; exit 1)

$(ELF): $(APP_OBJS) $(UDS_OBJS) diff.ld
	$(CC) $(CPUFLAGS) $(LDFLAGS) $(APP_OBJS) $(UDS_OBJS) -o $@

$(BUILD_DIR)/startup.o: ../startup.c | $(BUILD_DIR)
	$(CC) $(APP_CFLAGS) -c $< -o $@

$(BUILD_DIR)/main.o: main.c | $(BUILD_DIR)
	$(CC) $(APP_CFLAGS) -c $< -o $@

$(BUILD_DIR)/udslib/%.o: $(UDSLIB_DIR)/%.c | $(BUILD_DIR)
	@mkdir -p $(dir $@)
	$(CC) $(UDS_CFLAGS) -c $< -o $@

$(BUILD_DIR):
	mkdir -p $@

clean:
	rm -rf $(BUILD_DIR)

.PHONY: all check-udslib clean
