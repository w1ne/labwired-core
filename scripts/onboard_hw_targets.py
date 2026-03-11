import os
import re
import yaml
import shutil
import subprocess

LABWIRED_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONFIGS_DIR = os.path.join(LABWIRED_DIR, "core", "configs", "onboarding")
CHIPS_DIR = os.path.join(LABWIRED_DIR, "core", "configs", "chips", "onboarding")
SYSTEMS_DIR = os.path.join(LABWIRED_DIR, "core", "configs", "systems", "onboarding")
HW_SOURCE_DIR = "/tmp/hw-platforms"

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

def guess_arch(content, filename):
    lower_content = content.lower()
    if 'riscv64' in lower_content or 'rv64' in lower_content or 'mpfs' in filename.lower():
        return 'RISCV 64'
    elif 'riscv' in lower_content or 'rv32' in lower_content or 'minerva' in lower_content or 'vexriscv' in lower_content or 'sifive' in lower_content:
        return 'RISCV 32'
    elif 'cortex-a' in lower_content or 'zynqmp' in filename.lower():
        return 'ARM 64'
    elif 'cortex-m' in lower_content or 'stm32' in lower_content or 'nrf' in lower_content or 'sam' in lower_content or 'arm' in lower_content or 'nxp' in lower_content:
        return 'ARM 32'
    return 'Other'

def load_repl_with_includes(path, platforms_dir, visited=None):
    """Load a .repl file and recursively merge any included .repl files."""
    if visited is None:
        visited = set()
    if path in visited:
        return ""
    visited.add(path)

    try:
        with open(path, 'r', errors='replace') as f:
            content = f.read()
    except OSError:
        return ""

    # Resolve includes. Repl syntax variants:
    #   using "platforms/cpus/stm32f4.repl" (repo-root relative)
    #   using "./efm32g210.repl" (file relative)
    #   using @platforms/cpus/stm32f4.repl
    merged = content
    # Look for both quoted and @-prefixed includes
    for inc_match in re.finditer(r'using\s+(?:@|")([^"\n]+\.repl)"?', content):
        inc_path_str = inc_match.group(1)
        repo_root = os.path.dirname(platforms_dir)
        
        if inc_path_str.startswith("platforms/"):
            inc_abs = os.path.join(repo_root, inc_path_str)
        elif inc_path_str.startswith("./"):
            inc_abs = os.path.join(os.path.dirname(path), inc_path_str[2:])
        else:
            # Assume file relative if no prefix
            inc_abs = os.path.join(os.path.dirname(path), inc_path_str)
            
        if os.path.exists(inc_abs):
            merged += "\n" + load_repl_with_includes(inc_abs, platforms_dir, visited)

    return merged

ARCH_MAP = {
    'ARM': 'arm',
    'ARMv7-M': 'arm',
    'Cortex-M': 'arm',
    'Cortex-A': 'arm64',
    'RiscV': 'riscv',
    'RiscV64': 'riscv64',
    'X86': 'x86',
    'X86_64': 'x86_64',
    'MSP430': 'other',
}

FAMILIES = {
    'stm32': 'STM32', 'nucleo': 'STM32', 'discovery': 'STM32',
    'nrf': 'nRF5',
    'sam': 'Atmel SAM', 'atsam': 'Atmel SAM',
    'efm32': 'Silicon Labs EFM32', 'efr32': 'Silicon Labs EFR32', 'brd': 'Silicon Labs',
    'mimx': 'NXP i.MX', 'nxp': 'NXP', 'lpc': 'NXP LPC',
    'riscv': 'RISC-V Generic', 'sifive': 'SiFive', 'miv': 'Microchip Mi-V', 'vexriscv': 'LiteX/VexRiscV',
    'x86': 'x86 Generic', 'acrn': 'Intel ACRN',
    'leon3': 'SPARC Leon3',
}

CODE_EXAMPLES = {
    'STM32': '/* Zephyr Blinky for STM32 */\n#include <zephyr/kernel.h>\n#include <zephyr/drivers/gpio.h>\n\nvoid main(void) {\n    const struct gpio_dt_spec led = GPIO_DT_SPEC_GET(DT_ALIAS(led0), gpios);\n    gpio_pin_configure_dt(&led, GPIO_OUTPUT_ACTIVE);\n    while (1) {\n        gpio_pin_toggle_dt(&led);\n        k_msleep(500);\n    }\n}',
    'nRF5': '/* Zephyr Blinky for nRF52 */\n#include <zephyr/kernel.h>\n#include <zephyr/drivers/gpio.h>\n\nvoid main(void) {\n    const struct gpio_dt_spec led = GPIO_DT_SPEC_GET(DT_ALIAS(led0), gpios);\n    gpio_pin_configure_dt(&led, GPIO_OUTPUT_ACTIVE);\n    while (1) {\n        gpio_pin_toggle_dt(&led);\n        k_msleep(1000);\n    }\n}',
    'RISC-V Generic': '/* Minimal RISC-V Main */\nint main(void) {\n    char *uart = (char *)0x10000000;\n    const char *msg = "Hello RISC-V!\\n";\n    for (int i = 0; msg[i]; i++) *uart = msg[i];\n    while(1);\n    return 0;\n}',
    'x86 Generic': '/* Simple x86 Hello World */\n#include <stdio.h>\nint main(void) {\n    printf("LabWired x86 Boot Successful!\\n");\n    while(1);\n    return 0;\n}',
}

DEFAULT_CODE = '/* LabWired Hardware Initialization */\nint main(void) {\n    // Initialize hardware...\n    while(1) {\n        // Idle loop\n    }\n    return 0;\n}'

def parse_repl_file(content, filename):
    arch = guess_arch(content, filename)
    
    flash_base, flash_size = None, None
    ram_base, ram_size = None, None
    peripherals = []
    
    # 1. Parse RAM & Flash from Memory blocks
    # Support Memory.MappedMemory, Memory.ArrayMemory, etc.
    # Support both 'flash: Memory...' and '@ { bus 0x123; bus 0x456 }' multi-registration
    # Support both '@ sysbus' and '@sysbus'
    memory_matches = list(re.finditer(r'([a-zA-Z0-9_]+):\s+Memory\.(?:MappedMemory|ArrayMemory)\s+@\s*(?:sysbus\s+)?(?:{)?\s*(?:sysbus\s+)?([0-9a-fA-Fx]+)[\s\S]*?size:\s+([0-9a-fA-Fx]+)', content))
    for match in memory_matches:
        name = match.group(1).lower()
        base = match.group(2)
        size = match.group(3)
        
        # Priority matches for Flash/ROM
        if any(x in name for x in ['flash', 'rom', 'boot', 'code', 'itcm', 'bios', 'itim', 'dtim']):
            flash_base = flash_base or base
            flash_size = flash_size or size
        # Priority matches for RAM/DRAM
        elif any(x in name for x in ['ram', 'sram', 'memory', 'ddr', 'dtcm', 'sdram', 'lim', 'socsram']):
            ram_base = ram_base or base
            ram_size = ram_size or size

    # Fallback: if we still don't have flash/ram, just pick the first ones found
    if not flash_base and memory_matches:
        flash_base = memory_matches[0].group(2)
        flash_size = memory_matches[0].group(3)
    if not ram_base and len(memory_matches) > 1:
        ram_base = memory_matches[1].group(2)
        ram_size = memory_matches[1].group(3)

    # 2. Parse peripherals on any bus
    periph_blocks = re.finditer(r'([a-zA-Z0-9_]+):\s+([a-zA-Z0-9_]+\.[a-zA-Z0-9_]+)\s+@\s+([a-zA-Z0-9_]+)\s+(?:<)?([0-9a-fA-Fx]+)', content)
    seen_periph_ids = set()
    for match in periph_blocks:
        name = match.group(1)
        ptype = match.group(2)
        bus = match.group(3)
        base = match.group(4)
        if 'Memory.MappedMemory' not in ptype and 'CPU.' not in ptype and 'IRQControllers.' not in ptype:
            if name in seen_periph_ids:
                continue
            seen_periph_ids.add(name)
            try:
                base_address = int(base, 16) if base.startswith('0x') or any(c in 'abcdefABCDEF' for c in base) else int(base)
            except ValueError:
                base_address = 0
            
            peripherals.append({
                "id": name,
                "type": ptype.split('.')[-1].lower(),
                "bus": bus,
                "base_address": base_address
            })

    def safe_int(val):
        if val is None:
            return 0
        try:
            return int(val, 16) if isinstance(val, str) and (val.startswith('0x') or any(c in 'abcdefABCDEF' for c in val)) else int(val)
        except (ValueError, TypeError):
            return 0

    # Determine family
    family = 'Generic'
    fname_lower = filename.lower()
    for prefix, fam in FAMILIES.items():
        if fname_lower.startswith(prefix):
            family = fam
            break
            
    code_example = CODE_EXAMPLES.get(family, DEFAULT_CODE)

    return {
        "name": filename[:-5],
        "arch": ARCH_MAP.get(arch, 'arm'),
        "family": family,
        "code_example": code_example,
        "description": f"Hardware target. Family: {family}, Architecture: {arch}",
        "flash": {
            "base": safe_int(flash_base) or (0x08000000 if 'arm' in arch.lower() else 0),
            "size": f"{safe_int(flash_size)}B" if flash_size else "256KB",
        },
        "ram": {
            "base": safe_int(ram_base) or (0x20000000 if 'arm' in arch.lower() else 0),
            "size": f"{safe_int(ram_size)}B" if ram_size else "64KB",
        },
        "peripherals": peripherals
    }

def main():
    if not os.path.exists(HW_SOURCE_DIR):
        print("Fetching upstream hardware definitions...")
        subprocess.check_call(["git", "clone", "--depth", "1", "https://github.com/antmicro/renode.git", HW_SOURCE_DIR])
    
    os.makedirs(CONFIGS_DIR, exist_ok=True)
    os.makedirs(CHIPS_DIR, exist_ok=True)
    os.makedirs(SYSTEMS_DIR, exist_ok=True)
    
    platforms_dir = os.path.join(HW_SOURCE_DIR, "platforms")
    count = 0
    for root, _, files in os.walk(platforms_dir):
        for file in files:
            if file.endswith(".repl"):
                path = os.path.join(root, file)
                board = file[:-5]
                
                # Load with include resolution — boards inherit CPU memory maps
                content = load_repl_with_includes(path, platforms_dir)
                chip = parse_repl_file(content, file)

                # 1. Chip descriptor — the hardware model
                chip_yaml = {
                    "name": chip["name"],
                    "arch": chip["arch"],
                    "flash": chip["flash"],
                    "ram": chip["ram"],
                    "peripherals": chip["peripherals"],
                }
                write_yaml_with_header(os.path.join(CHIPS_DIR, f"{board}.yaml"), chip_yaml)

                # 2. System manifest — references the chip
                system_yaml = {
                    "name": board,
                    "chip": f"../../chips/onboarding/{board}.yaml",
                }
                write_yaml_with_header(os.path.join(SYSTEMS_DIR, f"{board}.yaml"), system_yaml)

                # 3. Catalog entry
                catalog_yaml = {
                    "name": board,
                    "description": chip["description"],
                    "family": chip["family"],
                    "code_example": chip["code_example"],
                    "pass_rate": 0,
                    "verified": False,
                    "system": f"systems/onboarding/{board}.yaml",
                }
                write_yaml_with_header(os.path.join(CONFIGS_DIR, f"{board}.yaml"), catalog_yaml)
                
                count += 1
                
if __name__ == "__main__":
    main()
