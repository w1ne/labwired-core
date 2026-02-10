//! SVD to Strict IR Transformation Logic

use super::*;
use svd_parser::svd::{self, Access, RegisterCluster};

impl IrDevice {
    /// Converts a parsed SVD Device into a LabWired IR Device (Strict/Flat).
    pub fn from_svd(svd: &svd::Device) -> Result<Self, String> {
        let mut peripherals = HashMap::new();
        let mut interrupt_mapping = HashMap::new();

        // Map SVD peripherals by name for lookup
        let svd_map: HashMap<String, &svd::Peripheral> = svd.peripherals.iter()
            .map(|p| (p.name.clone(), p))
            .collect();

        // Helper to resolve inheritance recursively
        fn resolve_peripheral(
            p: &svd::Peripheral,
            svd_map: &HashMap<String, &svd::Peripheral>,
            recursion_stack: &mut Vec<String>,
        ) -> Result<IrPeripheral, String> {
            if recursion_stack.contains(&p.name) {
                return Err(format!("Circular dependency in derivedFrom: {:?}", recursion_stack));
            }
            recursion_stack.push(p.name.clone());

            // 1. Start with Base (Parent or Empty)
            let mut resolved = if let Some(parent_name) = &p.derived_from {
                let parent_p = svd_map.get(parent_name)
                    .ok_or_else(|| format!("Peripheral {} derives from unknown {}", p.name, parent_name))?;

                let mut parent_resolved = resolve_peripheral(parent_p, svd_map, recursion_stack)?;

                // When deriving, the base address and name usually change to the derived one.
                // But the registers are copied.
                // We keep parent's registers and interrupts as a starting point.
                parent_resolved
            } else {
                IrPeripheral {
                    name: String::new(), // Will be overwritten
                    base_address: 0,
                    description: None,
                    registers: Vec::new(),
                    interrupts: Vec::new(),
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
                if let Some(existing_idx) = resolved.registers.iter().position(|r| r.name == local_reg.name) {
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
            description: Some(svd.description.clone()),
            peripherals,
            interrupt_mapping,
        })
    }

}

impl IrPeripheral {
    fn from_svd(p: &svd::Peripheral) -> Result<Self, String> {
        let mut registers = Vec::new();

        if let Some(regs) = &p.registers {
            for cluster_info in regs {
                flatten_cluster(cluster_info, 0, "", &mut registers)?;
            }
        }

        // Sort registers by offset for cleaner output/debugging
        registers.sort_by_key(|r| r.offset);

        // TODO: Handle derivedFrom at peripheral level if svd-parser doesn't fully resolve it.
        // svd-parser 0.14 generally keeps the structure. If we need to copy registers from another peripheral,
        // we'd need a second pass or access to the full device. For now assuming fully specified or pre-resolved.

        let interrupts = p.interrupt.iter().map(|i| IrInterrupt {
            name: i.name.clone(),
            description: i.description.clone(),
            value: i.value,
        }).collect();

        Ok(IrPeripheral {
            name: p.name.clone(),
            base_address: p.base_address,
            description: p.description.clone(),
            registers,
            interrupts,
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
                        let name = full_name_pattern.replace("[%s]", &idx_str).replace("%s", &idx_str);
                        // If no placeholder, append index (common SVD quirk)
                        let final_name = if name == full_name_pattern {
                            format!("{}{}", name, i)
                        } else {
                            name
                        };

                        let abs_offset = current_offset + info.address_offset as u64 + (i as u64 * stride);
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
                         let name = prefix_pattern.replace("[%s]", &idx_str).replace("%s", &idx_str);
                         let final_prefix = if name == prefix_pattern {
                             format!("{}_", name) // SVD naming is messy, usually cluster arrays act as struct arrays
                         } else {
                             format!("{}_", name)
                         };

                         let new_offset = current_offset + info.address_offset as u64 + (i as u64 * stride);
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
