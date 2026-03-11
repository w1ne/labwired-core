import os
import yaml

CONFIGS_DIR = "core/configs/onboarding"
TRACES_DIR = os.path.join(CONFIGS_DIR, "traces")

FAMILY_TRACES = {
    "STM32": "[0.000000] Booting STM32...\n[0.000120] RCC: Clock configuration successful\n[0.000450] GPIO: Port A initialized\n[0.000800] UART1: 115200 baud, 8N1\n[0.012000] LabWired: Hello from STM32 board!",
    "nRF5": "NRF52840 starting...\nSOFTDEVICE: Initializing...\nRADIO: Enabled\nUART: Powering on\nHello nRF52! [UART @ 0x40002000]",
    "Atmel SAM": "[BOARD] SAM-E70 Starting\n[CHIP] Flash 2MB / RAM 384KB OK\n[INIT] UART0 Initialized\nHello Atmel SAM!",
    "Silicon Labs EFM32": "[BOOT] EFM32GG Starting...\n[MEM] 1024KB Flash enabled\n[IO] USART1 Configured\nHello Silicon Labs!",
    "LiteX/VexRiscV": "-- LiteX BIOS --\n(c) Copyright 2012-2026 Enjoy-Digital\n(c) Copyright 2007-2026 M-Labs\n\nCPU: VexRiscv @ 100MHz\nRAM: 64MB (32-bit)\nHello LiteX!",
    "RISC-V Generic": "RISC-V Bootloader\nCPU: rv32imac\nRAM: 128MB\n[OK] UART initialized at 0x10000000\nHello RISC-V!",
    "x86 Generic": "SeaBIOS (version 1.16.0)\nBooting from Hard Disk...\nLabWired x86 Kernel Loaded.\n[OK] Video mode 1024x768\nHello x86!",
    "Intel ACRN": "ACRN Hypervisor Starting\nVersion: 3.0-rel\n[OK] CPU 0 Online\n[OK] VM 0 Started\nHello Intel ACRN!",
    "NXP i.MX": "U-Boot 2026.03 (Mar 11 2026)\nCPU: i.MX RT1064\nBoard: MIMXRT1064-EVK\n[OK] DRAM: 32MB\nHello NXP i.MX!",
}

LICENSE_HEADER = """# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.

"""

def write_yaml_with_header(path, data):
    with open(path, 'w') as f:
        f.write(LICENSE_HEADER)
        yaml.dump(data, f, default_flow_style=False, sort_keys=False)

def main():
    os.makedirs(TRACES_DIR, exist_ok=True)
    
    # 1. Create trace files
    for family, trace in FAMILY_TRACES.items():
        slug = family.lower().replace(" ", "_").replace("/", "_")
        path = os.path.join(TRACES_DIR, f"{slug}.txt")
        with open(path, 'w') as f:
            f.write(trace + "\n")
            
    # 2. Update catalog YAMLs
    for file in os.listdir(CONFIGS_DIR):
        if file.endswith(".yaml") and not os.path.isdir(os.path.join(CONFIGS_DIR, file)):
            path = os.path.join(CONFIGS_DIR, file)
            with open(path, 'r') as f:
                data = yaml.safe_load(f)
            
            family = data.get('family', 'Generic')
            slug = family.lower().replace(" ", "_").replace("/", "_")
            
            if family in FAMILY_TRACES:
                data['sample_trace'] = f"traces/{slug}.txt"
            else:
                # Create default trace for generic
                default_path = os.path.join(TRACES_DIR, "generic.txt")
                if not os.path.exists(default_path):
                    with open(default_path, 'w') as f:
                        f.write(DEFAULT_TRACE + "\n")
                data['sample_trace'] = "traces/generic.txt"
                
            write_yaml_with_header(path, data)
                
    print(f"Generated traces and updated catalog in {CONFIGS_DIR}")

if __name__ == "__main__":
    main()
