TARGET := h563_uds_ecu
BUILD_DIR := build
# Pinned UDSLib version. The build asserts UDS_VERSION_STR matches so a stale or
# mismatched checkout fails loudly instead of compiling against the wrong API
# (v2.0.0 renamed ctx->active_session to ctx->session.active, for one). Fetch a
# matching tree with `make fetch-udslib` (clones the tag into UDSLIB_DIR), or
# point UDSLIB_DIR at an existing v2.0.0 checkout.
UDSLIB_VERSION ?= 2.0.0
UDSLIB_REPO ?= https://github.com/w1ne/udslib.git
UDSLIB_DIR ?= /tmp/udslib-labwired-scope
CLANG ?= clang
RUST_LLD ?= $(HOME)/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld

CPUFLAGS := --target=arm-none-eabi -mcpu=cortex-m33 -mthumb
COMMON_CFLAGS := $(CPUFLAGS) -ffreestanding -fno-builtin -ffunction-sections \
                 -fdata-sections -Os -g -Iinclude -I$(UDSLIB_DIR)/include \
                 -I$(UDSLIB_DIR)/src/core -I../../common
VPATH = .:../../common
APP_CFLAGS := $(COMMON_CFLAGS) -Wall -Wextra -Werror
UDS_CFLAGS := $(COMMON_CFLAGS) -Wall -Wextra -Wno-unused-parameter \
              -Wno-missing-field-initializers
LDFLAGS := -flavor gnu -m armelf -T minimal.ld --gc-sections \
           -Map=$(BUILD_DIR)/$(TARGET).map

APP_SRCS := startup.c main.c uds_ecu_app.c
UDS_SRCS := \
  $(UDSLIB_DIR)/src/core/uds_core.c \
  $(UDSLIB_DIR)/src/services/uds_service_data.c \
  $(UDSLIB_DIR)/src/services/uds_service_flash.c \
  $(UDSLIB_DIR)/src/services/uds_service_io.c \
  $(UDSLIB_DIR)/src/services/uds_service_maintenance.c \
  $(UDSLIB_DIR)/src/services/uds_service_mem.c \
  $(UDSLIB_DIR)/src/services/uds_service_link.c \
  $(UDSLIB_DIR)/src/services/uds_service_roe.c \
  $(UDSLIB_DIR)/src/services/uds_service_security.c \
  $(UDSLIB_DIR)/src/services/uds_service_session.c \
  $(UDSLIB_DIR)/src/services/uds_dtc_store.c \
  $(UDSLIB_DIR)/src/transport/uds_tp_isotp.c

APP_OBJS := $(patsubst %.c,$(BUILD_DIR)/%.o,$(APP_SRCS))
UDS_OBJS := $(patsubst $(UDSLIB_DIR)/%.c,$(BUILD_DIR)/udslib/%.o,$(UDS_SRCS))
ELF := $(BUILD_DIR)/$(TARGET).elf

all: check-udslib $(ELF)

check-udslib:
	@test -f "$(UDSLIB_DIR)/include/uds/uds_core.h" || \
	  (echo "UDSLIB_DIR must point at a UDSLib checkout (try: make fetch-udslib)" >&2; exit 1)
	@grep -q 'UDS_VERSION_STR "$(UDSLIB_VERSION)"' "$(UDSLIB_DIR)/include/uds/uds_version.h" || \
	  (echo "UDSLIB_DIR is not UDSLib v$(UDSLIB_VERSION) (expected UDS_VERSION_STR \"$(UDSLIB_VERSION)\" in uds_version.h); run 'make fetch-udslib' or repoint UDSLIB_DIR" >&2; exit 1)

# Clone the pinned tag into UDSLIB_DIR when it is absent (reproducible build).
fetch-udslib:
	@if [ ! -f "$(UDSLIB_DIR)/include/uds/uds_version.h" ]; then \
	  echo "Cloning UDSLib v$(UDSLIB_VERSION) into $(UDSLIB_DIR)"; \
	  git clone --branch "v$(UDSLIB_VERSION)" --depth 1 "$(UDSLIB_REPO)" "$(UDSLIB_DIR)"; \
	else \
	  echo "$(UDSLIB_DIR) already populated; skipping clone"; \
	fi

$(ELF): $(APP_OBJS) $(UDS_OBJS) minimal.ld
	$(RUST_LLD) $(LDFLAGS) $(APP_OBJS) $(UDS_OBJS) -o $@

$(BUILD_DIR)/%.o: %.c | $(BUILD_DIR)
	$(CLANG) $(APP_CFLAGS) -c $< -o $@

$(BUILD_DIR)/udslib/%.o: $(UDSLIB_DIR)/%.c | $(BUILD_DIR)
	@mkdir -p $(dir $@)
	$(CLANG) $(UDS_CFLAGS) -c $< -o $@

$(BUILD_DIR):
	mkdir -p $@

clean:
	rm -rf $(BUILD_DIR)

.PHONY: all check-udslib fetch-udslib clean
