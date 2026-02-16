SHELL := /usr/bin/env bash

EXAMPLE_DIR := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
CORE_DIR := $(abspath $(EXAMPLE_DIR)/../..)

TARGET_TRIPLE ?= thumbv7m-none-eabi
PROFILE ?= debug
FIRMWARE_PKG ?= firmware-h563-io-demo

CORE_ELF := $(CORE_DIR)/target/$(TARGET_TRIPLE)/$(PROFILE)/$(FIRMWARE_PKG)
LOCAL_TARGET_DIR := $(EXAMPLE_DIR)/target
LOCAL_ELF := $(LOCAL_TARGET_DIR)/firmware

.PHONY: all clean FORCE

all: $(LOCAL_ELF)

$(LOCAL_ELF): FORCE
	cd "$(CORE_DIR)" && \
		cargo build -p "$(FIRMWARE_PKG)" --target "$(TARGET_TRIPLE)" $(if $(filter $(PROFILE),release),--release,)
	mkdir -p "$(LOCAL_TARGET_DIR)"
	cp "$(CORE_ELF)" "$(LOCAL_ELF)"

FORCE:

clean:
	rm -rf "$(LOCAL_TARGET_DIR)"
