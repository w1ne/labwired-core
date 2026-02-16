//! SVD to Strict IR Transformation Logic

use super::*;
use svd_parser::svd::{self, Access, RegisterCluster};

impl IrDevice {
    /// Converts a parsed SVD Device into a LabWired IR Device (Strict/Flat).
    pub fn from_svd(svd: &svd::Device) -> Result<Self, String> {
        let mut peripherals = HashMap::new();
        let mut interrupt_mapping = HashMap::new();

        let arch = if let Some(cpu) = &svd.cpu {
            cpu.name.clone()
        } else {
            "Unknown".to_string()
        };

        // Map SVD peripherals by name for lookup
        let svd_map: HashMap<String, &svd::Peripheral> = svd
            .peripherals
            .iter()
            .map(|p| (p.name.clone(), p))
            .collect();

        // Helper to resolve inheritance recursively
        fn resolve_peripheral(
            p: &svd::Peripheral,
            svd_map: &HashMap<String, &svd::Peripheral>,
            recursion_stack: &mut Vec<String>,
        ) -> Result<IrPeripheral, String> {
            if recursion_stack.contains(&p.name) {
                return Err(format!(
                    "Circular dependency in derivedFrom: {:?}",
                    recursion_stack
                ));
            }
            recursion_stack.push(p.name.clone());

            // 1. Start with Base (Parent or Empty)
            let mut resolved = if let Some(parent_name) = &p.derived_from {
                let parent_p = svd_map.get(parent_name).ok_or_else(|| {
                    format!("Peripheral {} derives from unknown {}", p.name, parent_name)
                })?;

                resolve_peripheral(parent_p, svd_map, recursion_stack)?
            } else {
                IrPeripheral {
                    name: String::new(), // Will be overwritten
                    base_address: 0,
                    description: None,
                    registers: Vec::new(),
                    interrupts: Vec::new(),
                    timing: Vec::new(),
                }
            };

            // 2. Apply Local Overrides
            // "The derivedFrom attribute specifies that the peripheral is a copy...
            //  modified by the elements specified in this peripheral."

            resolved.name = p.name.clone();
            resolved.base_address = p.base_address;
            if let Some(d) = &p.description {
                resolved.description = Some(d.clone());
            }

            // 3. Merge Registers
            // "Registers ... are added... If a register with the same name exists... it is redefined"
            let mut local_registers = Vec::new();
            if let Some(regs) = &p.registers {
                for cluster_info in regs {
                    flatten_cluster(cluster_info, 0, "", &mut local_registers)?;
                }
            }

            for local_reg in local_registers {
                if let Some(existing_idx) = resolved
                    .registers
                    .iter()
                    .position(|r| r.name == local_reg.name)
                {
                    // Redefine/Overwrite
                    // Note: SVD spec implies redefinition. With strict IR we just replace the struct.
                    // However, we must ensure offsets are correct relative to the NEW base address?
                    // The offsets in IrRegister are relative to peripheral base.
                    // So they are portable.
                    resolved.registers[existing_idx] = local_reg;
                } else {
                    // Append
                    resolved.registers.push(local_reg);
                }
            }

            // 4. Merge Interrupts
            for i in &p.interrupt {
                let ir_intr = IrInterrupt {
                    name: i.name.clone(),
                    description: i.description.clone(),
                    value: i.value,
                };
                // Check if interrupt name exists? or just append?
                // Interrupts are usually additive or remapped.
                // We'll append for now.
                resolved.interrupts.push(ir_intr);
            }

            recursion_stack.pop();
            Ok(resolved)
        }

        for p in &svd.peripherals {
            let resolved = resolve_peripheral(p, &svd_map, &mut Vec::new())?;
            peripherals.insert(resolved.name.clone(), resolved);

            // Populate global interrupt mapping
            for intr in &p.interrupt {
                interrupt_mapping.insert(intr.name.clone(), intr.value);
            }
        }

        // Final cleanup: Sort registers one last time for deterministic output
        for p in peripherals.values_mut() {
            p.registers.sort_by_key(|r| r.offset);
        }

        Ok(IrDevice {
            name: svd.name.clone(),
            arch,
            description: Some(svd.description.clone()),
            peripherals,
            interrupt_mapping,
        })
    }
}

/// Recursively flattens SVD clusters and arrays into a simple list of registers.
fn flatten_cluster(
    rc: &RegisterCluster,
    current_offset: u64,
    name_prefix: &str,
    out: &mut Vec<IrRegister>,
) -> Result<(), String> {
    match rc {
        RegisterCluster::Register(reg) => {
            // Handle Register Arrays and Single Registers
            match reg {
                svd::Register::Single(info) => {
                    let full_name = format!("{}{}", name_prefix, info.name);
                    let abs_offset = current_offset + info.address_offset as u64;
                    out.push(convert_register(info, &full_name, abs_offset));
                }
                svd::Register::Array(info, dim) => {
                    let full_name_pattern = format!("{}{}", name_prefix, info.name);
                    // info.name usually contains "[%s]"

                    let stride = dim.dim_increment as u64;
                    for i in 0..dim.dim {
                        let idx_str = i.to_string();
                        // simplistic placeholder replacement
                        let name = full_name_pattern
                            .replace("[%s]", &idx_str)
                            .replace("%s", &idx_str);
                        // If no placeholder, append index (common SVD quirk)
                        let final_name = if name == full_name_pattern {
                            format!("{}{}", name, i)
                        } else {
                            name
                        };

                        let abs_offset =
                            current_offset + info.address_offset as u64 + (i as u64 * stride);
                        out.push(convert_register(info, &final_name, abs_offset));
                    }
                }
            }
        }
        RegisterCluster::Cluster(cluster) => {
            // Handle Cluster Arrays and Single Clusters
            match cluster {
                svd::Cluster::Single(info) => {
                    let new_prefix = format!("{}{}_", name_prefix, info.name);
                    let new_offset = current_offset + info.address_offset as u64;
                    for child in &info.children {
                        flatten_cluster(child, new_offset, &new_prefix, out)?;
                    }
                }
                svd::Cluster::Array(info, dim) => {
                    let stride = dim.dim_increment as u64;
                    let prefix_pattern = format!("{}{}", name_prefix, info.name);

                    for i in 0..dim.dim {
                        let idx_str = i.to_string();
                        let name = prefix_pattern
                            .replace("[%s]", &idx_str)
                            .replace("%s", &idx_str);
                        let final_prefix = format!("{}_", name);

                        let new_offset =
                            current_offset + info.address_offset as u64 + (i as u64 * stride);
                        for child in &info.children {
                            flatten_cluster(child, new_offset, &final_prefix, out)?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn convert_register(info: &svd::RegisterInfo, name: &str, offset: u64) -> IrRegister {
    let mut fields = Vec::new();
    if let Some(fs) = &info.fields {
        for f in fs {
            fields.push(IrField {
                name: f.name.clone(),
                bit_offset: f.bit_range.offset,
                bit_width: f.bit_range.width,
                access: Some(map_access(f.access)),
                description: f.description.clone(),
            });
        }
    }

    // Sort fields by bit offset
    fields.sort_by_key(|f| f.bit_offset);

    IrRegister {
        name: name.to_string(),
        offset,
        size: info.properties.size.unwrap_or(32),
        access: map_access(info.properties.access),
        reset_value: info.properties.reset_value.unwrap_or(0),
        fields,
        side_effects: None,
        description: info.description.clone(),
    }
}

fn map_access(access: Option<Access>) -> IrAccess {
    match access {
        Some(Access::ReadOnly) => IrAccess::ReadOnly,
        Some(Access::WriteOnly) => IrAccess::WriteOnly,
        Some(Access::ReadWrite) => IrAccess::ReadWrite,
        Some(Access::WriteOnce) => IrAccess::WriteOnce,
        Some(Access::ReadWriteOnce) => IrAccess::ReadWriteOnce,
        None => IrAccess::ReadWrite, // Default reasonable assumption
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use svd_parser::svd::{self, Register, RegisterCluster};

    fn create_mock_register(name: &str, offset: u32) -> Register {
        Register::Single(
            svd::RegisterInfo::builder()
                .name(name.to_string())
                .address_offset(offset)
                .build(svd::ValidateLevel::Disabled)
                .unwrap(),
        )
    }

    fn create_mock_peripheral(name: &str, base: u64) -> svd::Peripheral {
        svd::Peripheral::Single(
            svd::PeripheralInfo::builder()
                .name(name.to_string())
                .base_address(base)
                .registers(Some(vec![]))
                .build(svd::ValidateLevel::Disabled)
                .unwrap(),
        )
    }

    #[test]
    fn test_flatten_basic_register() {
        let mut out = Vec::new();
        let reg = create_mock_register("CR", 0x00);
        let rc = RegisterCluster::Register(reg);

        flatten_cluster(&rc, 0, "", &mut out).unwrap();

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "CR");
        assert_eq!(out[0].offset, 0x00);
    }

    #[test]
    fn test_flatten_register_array() {
        let mut out = Vec::new();
        let info = svd::RegisterInfo::builder()
            .name("REG[%s]".to_string())
            .address_offset(0x00)
            .build(svd::ValidateLevel::Disabled)
            .unwrap();
        let dim = svd::DimElement::builder()
            .dim(3)
            .dim_increment(0x4)
            .build(svd::ValidateLevel::Disabled)
            .unwrap();

        let rc = RegisterCluster::Register(Register::Array(info, dim));
        flatten_cluster(&rc, 0x1000, "PERIPH_", &mut out).unwrap();

        assert_eq!(out.len(), 3);
        assert_eq!(out[0].name, "PERIPH_REG0");
        assert_eq!(out[0].offset, 0x1000); // 0x1000 + 0
        assert_eq!(out[1].name, "PERIPH_REG1");
        assert_eq!(out[1].offset, 0x1004); // 0x1000 + 4
        assert_eq!(out[2].name, "PERIPH_REG2");
        assert_eq!(out[2].offset, 0x1008); // 0x1000 + 8
    }

    #[test]
    fn test_inheritance_resolution() {
        // Mock SVD structure
        // PARENT: [CR @ 0x00]
        // CHILD matches PARENT but adds [SR @ 0x04]

        let mut parent = create_mock_peripheral("PARENT", 0x1000);
        parent.registers = Some(vec![RegisterCluster::Register(create_mock_register(
            "CR", 0x00,
        ))]);

        let mut child = create_mock_peripheral("CHILD", 0x2000);
        child.derived_from = Some("PARENT".to_string());
        child.registers = Some(vec![RegisterCluster::Register(create_mock_register(
            "SR", 0x04,
        ))]);

        let device = svd::Device::builder()
            .name("TEST".to_string())
            .peripherals(vec![parent, child])
            .description(String::new())
            .address_unit_bits(8)
            .width(32)
            .build(svd::ValidateLevel::Disabled)
            .unwrap();

        let ir = IrDevice::from_svd(&device).unwrap();

        let child_ir = &ir.peripherals["CHILD"];
        assert_eq!(child_ir.base_address, 0x2000);
        assert_eq!(child_ir.registers.len(), 2);

        let cr = child_ir
            .registers
            .iter()
            .find(|r| r.name == "CR")
            .expect("CR not found in CHILD");
        assert_eq!(cr.offset, 0x00); // Relative offset preserved

        let sr = child_ir
            .registers
            .iter()
            .find(|r| r.name == "SR")
            .expect("SR not found in CHILD");
        assert_eq!(sr.offset, 0x04);
    }
}
