# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: GPL-3.0-or-later

cmake_minimum_required(VERSION 3.20.0)

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(iolink_master)

target_sources(app PRIVATE src/main.c)
