import os
import json

renode_path = "/opt/renode/platforms"
boards_path = os.path.join(renode_path, "boards")
cpus_path = os.path.join(renode_path, "cpus")

tier1_boards = {
    "stm32f4_discovery.repl", "stm32f7_discovery-bb.repl", "stm32f072b_discovery.repl",
    "arduino_nano_33_ble.repl", "arty_litex_vexriscv.repl", "beaglev_starlight.repl",
    "silabs/brd4162a.repl", "silabs/slwstk6220a.repl"
}
tier1_cpus = {
    "stm32f103.repl", "stm32f4.repl", "stm32g0.repl", "stm32h743.repl", "stm32l071.repl",
    "nrf52840.repl", "atsamd51g19a.repl", "sam_e70.repl", "sifive-fe310.repl",
    "nxp-k6xf.repl", "imxrt1064.repl", "riscv_virt.repl", "litex_vexriscv.repl"
}

def get_name(rel_path):
    base = os.path.basename(rel_path)
    return os.path.splitext(base)[0]

hardware = []

for root, dirs, files in os.walk(boards_path):
    for file in files:
        if file.endswith(".repl"):
            full_path = os.path.join(root, file)
            rel_path = os.path.relpath(full_path, boards_path)
            # normalize for exact matching
            rel_path_norm = rel_path.replace("\\", "/")

            tier = 1 if rel_path_norm in tier1_boards else 2
            hardware.append({
                "id": f"board-{get_name(rel_path)}",
                "name": get_name(rel_path),
                "type": "board",
                "repl_path": f"platforms/boards/{rel_path_norm}",
                "tier": tier
            })

for root, dirs, files in os.walk(cpus_path):
    for file in files:
        if file.endswith(".repl"):
            full_path = os.path.join(root, file)
            rel_path = os.path.relpath(full_path, cpus_path)
            rel_path_norm = rel_path.replace("\\", "/")

            tier = 1 if rel_path_norm in tier1_cpus else 2
            hardware.append({
                "id": f"cpu-{get_name(rel_path)}",
                "name": get_name(rel_path),
                "type": "cpu",
                "repl_path": f"platforms/cpus/{rel_path_norm}",
                "tier": tier
            })

hardware.sort(key=lambda x: (x["tier"], x["type"], x["name"]))

output_file = "/home/andrii/Projects/labwired/foundry/backend/configs/renode_hardware.json"
with open(output_file, 'w') as f:
    json.dump(hardware, f, indent=2)

print(f"Generated {len(hardware)} entries in {output_file}")
