#![deny(missing_docs)]

//! # SVD Ingestor
//!
//! A tool for converting CMSIS-SVD files into LabWired `PeripheralDescriptor` YAML files.
//! This crate provides the core logic for parsing and conversion.

use labwired_config::{Access, FieldDescriptor, PeripheralDescriptor, RegisterDescriptor};
use std::collections::HashMap;
use std::path::Path;
use svd_parser::svd::{Access as SvdAccess, Device, Field, Peripheral, Register, RegisterCluster};
use thiserror::Error;

/// Errors that can occur during SVD ingestion and conversion.
#[derive(Error, Debug)]
pub enum IngestError {
    /// IO error during file reading or writing.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// Error serializing the output YAML.
    #[error("YAML serialization error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    /// Error parsing the SVD file content.
    #[error("SVD parsing error: {0}")]
    Svd(String),
    /// The SVD structure is invalid or unsupported.
    #[error("Invalid SVD structure: {0}")]
    InvalidSvd(String),
}

/// Result type for SVD ingestor operations.
pub type Result<T> = std::result::Result<T, IngestError>;

/// Processes a single peripheral from the SVD device and converts it into a `PeripheralDescriptor`.
///
/// This function handles:
/// - Flattening nested register clusters.
/// - Unrolling register arrays.
/// - Extracting interrupt information.
/// - sorting registers by address offset.
///
/// # Arguments
/// * `_device` - The parent SVD device (currently unused but reserved for context).
/// * `peripheral` - The SVD peripheral to process.
pub fn process_peripheral(
    device: &Device,
    peripheral: &Peripheral,
) -> Result<PeripheralDescriptor> {
    let mut registers = Vec::new();
    let mut interrupts = HashMap::new();

    let p_info = match peripheral {
        Peripheral::Single(info) => info,
        Peripheral::Array(info, _dim) => info,
    };

    let mut current_peripheral = p_info;
    let mut peripherals_to_process = vec![p_info];

    while let Some(base_name) = &current_peripheral.derived_from {
        let base = device
            .peripherals
            .iter()
            .find(|p| match p {
                Peripheral::Single(info) => &info.name == base_name,
                Peripheral::Array(info, _dim) => &info.name == base_name,
            })
            .ok_or_else(|| {
                IngestError::InvalidSvd(format!("Base peripheral {} not found", base_name))
            })?;

        let base_info = match base {
            Peripheral::Single(info) => info,
            Peripheral::Array(info, _dim) => info,
        };
        peripherals_to_process.push(base_info);
        current_peripheral = base_info;
    }

    // Build register lookup map for derivedFrom resolution
    let mut reg_map = HashMap::new();
    for p in &peripherals_to_process {
        if let Some(children) = &p.registers {
            for cluster in children {
                collect_all_registers(cluster, &mut reg_map);
            }
        }
    }

    // Process from most-base to derived
    for p in peripherals_to_process.iter().rev() {
        if let Some(children) = &p.registers {
            for cluster in children {
                process_register_cluster(cluster, &mut registers, 0, "", &reg_map)?;
            }
        }
    }

    // Deduplicate by name, keeping latest (most derived)
    let mut unique_regs: HashMap<String, RegisterDescriptor> = HashMap::new();
    for reg in registers {
        unique_regs.insert(reg.id.clone(), reg);
    }
    let mut registers: Vec<_> = unique_regs.into_values().collect();

    registers.sort_by_key(|r| r.address_offset);

    for p in peripherals_to_process.iter().rev() {
        for interrupt in &p.interrupt {
            interrupts.insert(interrupt.name.clone(), interrupt.value);
        }
    }

    Ok(PeripheralDescriptor {
        peripheral: p_info.name.clone(),
        version: "0.1.0".to_string(),
        registers,
        interrupts: if interrupts.is_empty() {
            None
        } else {
            Some(interrupts)
        },
        timing: None,
    })
}

fn collect_all_registers<'a>(
    cluster: &'a RegisterCluster,
    map: &mut HashMap<String, &'a svd_parser::svd::RegisterInfo>,
) {
    match cluster {
        RegisterCluster::Register(reg) => match reg {
            Register::Single(info) => {
                map.insert(info.name.clone(), info);
            }
            Register::Array(info, _dim) => {
                map.insert(info.name.clone(), info);
            }
        },
        RegisterCluster::Cluster(cluster) => match cluster {
            svd_parser::svd::Cluster::Single(info) => {
                for child in &info.children {
                    collect_all_registers(child, map);
                }
            }
            svd_parser::svd::Cluster::Array(info, _dim) => {
                for child in &info.children {
                    collect_all_registers(child, map);
                }
            }
        },
    }
}

fn process_register_cluster(
    cluster: &RegisterCluster,
    registers: &mut Vec<RegisterDescriptor>,
    parent_offset: u64,
    name_prefix: &str,
    reg_map: &HashMap<String, &svd_parser::svd::RegisterInfo>,
) -> Result<()> {
    match cluster {
        RegisterCluster::Register(reg) => {
            expand_register(reg, registers, parent_offset, name_prefix, reg_map)?;
        }
        RegisterCluster::Cluster(cluster) => {
            expand_cluster(cluster, registers, parent_offset, name_prefix, reg_map)?;
        }
    }
    Ok(())
}

fn expand_register(
    reg_array: &Register,
    registers: &mut Vec<RegisterDescriptor>,
    parent_offset: u64,
    name_prefix: &str,
    reg_map: &HashMap<String, &svd_parser::svd::RegisterInfo>,
) -> Result<()> {
    match reg_array {
        Register::Single(reg) => {
            if let Some(mut desc) = convert_register(reg, parent_offset, reg_map)? {
                if !name_prefix.is_empty() {
                    desc.id = format!("{}_{}", name_prefix, desc.id);
                }
                registers.push(desc);
            }
        }
        Register::Array(reg, dim) => {
            for i in 0..dim.dim {
                let mut name = replace_dim_name(&reg.name, i, dim);
                if !name_prefix.is_empty() {
                    name = format!("{}_{}", name_prefix, name);
                }
                let offset = parent_offset
                    + (reg.address_offset as u64)
                    + (i as u64 * dim.dim_increment as u64);

                if let Some(mut desc) = convert_register(reg, 0, reg_map)? {
                    desc.id = name;
                    desc.address_offset = offset;
                    registers.push(desc);
                }
            }
        }
    }
    Ok(())
}

fn expand_cluster(
    cluster_array: &svd_parser::svd::Cluster,
    registers: &mut Vec<RegisterDescriptor>,
    parent_offset: u64,
    name_prefix: &str,
    reg_map: &HashMap<String, &svd_parser::svd::RegisterInfo>,
) -> Result<()> {
    match cluster_array {
        svd_parser::svd::Cluster::Single(cluster) => {
            let current_offset = parent_offset + cluster.address_offset as u64;
            let cluster_name = if !name_prefix.is_empty() {
                format!("{}_{}", name_prefix, cluster.name)
            } else {
                cluster.name.clone()
            };

            for child in &cluster.children {
                process_register_cluster(child, registers, current_offset, &cluster_name, reg_map)?;
            }
        }
        svd_parser::svd::Cluster::Array(cluster, dim) => {
            for i in 0..dim.dim {
                let current_offset = parent_offset
                    + (cluster.address_offset as u64)
                    + (i as u64 * dim.dim_increment as u64);

                let instance_name = replace_dim_name(&cluster.name, i, dim);
                let cluster_name = if !name_prefix.is_empty() {
                    format!("{}_{}", name_prefix, instance_name)
                } else {
                    instance_name
                };

                for child in &cluster.children {
                    process_register_cluster(
                        child,
                        registers,
                        current_offset,
                        &cluster_name,
                        reg_map,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn replace_dim_name(name: &str, index: u32, dim: &svd_parser::svd::DimElement) -> String {
    let index_str = if let Some(dim_index) = &dim.dim_index {
        // Handle dimIndex array/list (n/a for simple MVP, usually just use 'i')
        // svd-parser returns valid Vec<String> for indexes().
        // Let's rely on simple string formatting if index list is complex.
        // Actually svd-rs has helper for this.
        // But implementing simple "%s" replacement is safer for now.
        if index < dim_index.len() as u32 {
            dim_index[index as usize].clone()
        } else {
            index.to_string()
        }
    } else {
        index.to_string()
    };

    name.replace("[%s]", &index_str).replace("%s", &index_str)
}

fn convert_register(
    reg: &svd_parser::svd::RegisterInfo,
    extra_offset: u64,
    reg_map: &HashMap<String, &svd_parser::svd::RegisterInfo>,
) -> Result<Option<RegisterDescriptor>> {
    let mut current_reg = reg;
    if let Some(base_name) = &reg.derived_from {
        if let Some(base) = reg_map.get(base_name) {
            current_reg = base;
        } else {
            return Err(IngestError::InvalidSvd(format!(
                "Base register {} not found",
                base_name
            )));
        }
    }

    let offset = extra_offset + reg.address_offset as u64;

    let size = reg
        .properties
        .size
        .or(current_reg.properties.size)
        .unwrap_or(32) as u8;

    let access = match reg.properties.access.or(current_reg.properties.access) {
        Some(SvdAccess::ReadWrite) => Access::ReadWrite,
        Some(SvdAccess::ReadOnly) => Access::ReadOnly,
        Some(SvdAccess::WriteOnly) => Access::WriteOnly,
        None => Access::ReadWrite,
        _ => Access::ReadWrite,
    };

    let reset_value = reg
        .properties
        .reset_value
        .or(current_reg.properties.reset_value)
        .unwrap_or(0) as u32;

    let mut fields = Vec::new();
    let mut field_map = HashMap::new();

    if let Some(svd_fields) = &current_reg.fields {
        for field in svd_fields {
            match field {
                Field::Single(f) => {
                    field_map.insert(f.name.clone(), f.clone());
                }
                Field::Array(f, dim) => {
                    for i in 0..dim.dim {
                        let mut f_clone = f.clone();
                        f_clone.name = replace_dim_name(&f.name, i, dim);
                        let shift = i * dim.dim_increment;
                        f_clone.bit_range.offset = f.bit_range.offset + shift;
                        field_map.insert(f_clone.name.clone(), f_clone);
                    }
                }
            }
        }
    }

    if let Some(svd_fields) = &reg.fields {
        for field in svd_fields {
            match field {
                Field::Single(f) => {
                    field_map.insert(f.name.clone(), f.clone());
                }
                Field::Array(f, dim) => {
                    for i in 0..dim.dim {
                        let mut f_clone = f.clone();
                        f_clone.name = replace_dim_name(&f.name, i, dim);
                        let shift = i * dim.dim_increment;
                        f_clone.bit_range.offset = f.bit_range.offset + shift;
                        field_map.insert(f_clone.name.clone(), f_clone);
                    }
                }
            }
        }
    }

    let mut field_items: Vec<_> = field_map.into_values().collect();
    field_items.sort_by_key(|f| f.bit_range.offset);
    for f in field_items {
        fields.push(convert_field(&f)?);
    }

    let mut side_effects = labwired_config::SideEffectsDescriptor {
        read_action: None,
        write_action: None,
        on_read: None,
        on_write: None,
    };

    let reg_read_action = reg.read_action.or(current_reg.read_action);
    if let Some(svd_parser::svd::ReadAction::Clear) = reg_read_action {
        side_effects.read_action = Some(labwired_config::ReadAction::Clear);
    }

    let reg_modified_write = reg
        .modified_write_values
        .or(current_reg.modified_write_values);
    if let Some(svd_write) = reg_modified_write {
        match svd_write {
            svd_parser::svd::ModifiedWriteValues::OneToClear => {
                side_effects.write_action = Some(labwired_config::WriteAction::WriteOneToClear);
            }
            svd_parser::svd::ModifiedWriteValues::ZeroToClear => {
                side_effects.write_action = Some(labwired_config::WriteAction::WriteZeroToClear);
            }
            _ => {}
        }
    }

    let side_effects = if side_effects.read_action.is_some() || side_effects.write_action.is_some()
    {
        Some(side_effects)
    } else {
        None
    };

    Ok(Some(RegisterDescriptor {
        id: reg.name.clone(),
        address_offset: offset,
        size,
        access,
        reset_value,
        fields,
        side_effects,
    }))
}

fn convert_field(field: &svd_parser::svd::FieldInfo) -> Result<FieldDescriptor> {
    let msb = field.bit_range.offset + field.bit_range.width - 1;
    let lsb = field.bit_range.offset;

    Ok(FieldDescriptor {
        name: field.name.clone(),
        bit_range: [msb as u8, lsb as u8],
        description: field.description.clone(),
    })
}

/// Saves the generated `PeripheralDescriptor` to a YAML file in the specified output directory.
///
/// The filename will be `{peripheral_name}.yaml` (lowercase).
///
/// # Arguments
/// * `descriptor` - The descriptor to save.
/// * `output_dir` - The target directory path.
pub fn save_descriptor(descriptor: &PeripheralDescriptor, output_dir: &Path) -> Result<()> {
    let yaml = serde_yaml::to_string(descriptor)?;
    let filename = output_dir.join(format!("{}.yaml", descriptor.peripheral.to_lowercase()));
    std::fs::write(&filename, yaml)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use svd_parser::svd::BitRangeType;
    use svd_parser::svd::{BitRange, FieldInfo, PeripheralInfo, RegisterCluster};

    fn make_register(name: &str, offset: u32) -> Register {
        let mut reg = svd_parser::svd::RegisterInfo::builder().name(name.to_string());
        reg = reg.address_offset(offset);
        let mut info = reg.build(svd_parser::ValidateLevel::Disabled).unwrap();
        // properties are default, modify them
        info.properties.size = Some(32);
        info.properties.access = Some(SvdAccess::ReadWrite);
        info.properties.reset_value = Some(0);
        Register::Single(info)
    }

    #[test]
    fn test_register_conversion() {
        let reg_enum = make_register("TEST_REG", 0x10);
        if let Register::Single(reg) = reg_enum {
            let reg_map = HashMap::new();
            let desc = convert_register(&reg, 0, &reg_map).unwrap().unwrap();
            assert_eq!(desc.id, "TEST_REG");
            assert_eq!(desc.address_offset, 0x10);
            assert_eq!(desc.size, 32);
            assert_eq!(desc.access, Access::ReadWrite);
        } else {
            panic!("Expected Single register");
        }
    }

    #[test]
    fn test_field_conversion() {
        let mut field = FieldInfo::builder().name("TEST_FIELD".to_string());
        field = field.bit_range(BitRange {
            offset: 4,
            width: 4, // 4..8 -> bits 4,5,6,7. MSB=7, LSB=4
            range_type: BitRangeType::BitRange,
        });

        let field = field.build(svd_parser::ValidateLevel::Disabled).unwrap();
        // let field_enum = Field::Single(field); // No longer needed
        let desc = convert_field(&field).unwrap();

        assert_eq!(desc.name, "TEST_FIELD");
        assert_eq!(desc.bit_range, [7, 4]); // [msb, lsb]
    }

    #[test]
    fn test_single_bit_field() {
        let mut field = FieldInfo::builder().name("BIT".to_string());
        field = field.bit_range(BitRange {
            offset: 0,
            width: 1, // 0..1 -> bit 0. MSB=0, LSB=0
            range_type: BitRangeType::BitRange,
        });

        let field = field.build(svd_parser::ValidateLevel::Disabled).unwrap();
        // let field_enum = Field::Single(field);
        let desc = convert_field(&field).unwrap();

        assert_eq!(desc.bit_range, [0, 0]);
    }

    // Integration test style unit test involving Peripheral structure
    #[test]
    fn test_peripheral_processing() {
        let mut p = PeripheralInfo::builder()
            .name("UART".to_string())
            .base_address(0x4000_0000);

        // Add a register
        let mut reg = make_register("CR1", 0x00);
        let mut field = FieldInfo::builder().name("EN".to_string());
        field = field.bit_range(BitRange {
            offset: 0,
            width: 1,
            range_type: BitRangeType::BitRange,
        });

        // We need to inject field into register. Register is an Enum (Single/Array).
        // make_register returns Register::Single(info).
        if let Register::Single(ref mut info) = reg {
            info.fields = Some(vec![Field::Single(
                field.build(svd_parser::ValidateLevel::Disabled).unwrap(),
            )]);
        }

        p = p.registers(Some(vec![RegisterCluster::Register(reg)]));

        let peripheral = p.build(svd_parser::ValidateLevel::Disabled).unwrap();
        let peripheral_enum = Peripheral::Single(peripheral);
        let device = Device::builder()
            .name("STM32".to_string())
            .peripherals(vec![])
            .build(svd_parser::ValidateLevel::Disabled)
            .unwrap();

        let desc = process_peripheral(&device, &peripheral_enum).unwrap();

        assert_eq!(desc.peripheral, "UART");
        assert_eq!(desc.registers.len(), 1);
        assert_eq!(desc.registers[0].id, "CR1");
        assert_eq!(desc.registers[0].fields.len(), 1);
        assert_eq!(desc.registers[0].fields[0].name, "EN");
    }
    #[test]
    fn test_cluster_and_array_processing() {
        let mut p = PeripheralInfo::builder()
            .name("GPIO".to_string())
            .base_address(0x4001_0800);

        // Create an array of registers: GPIOx_CRL, GPIOx_CRH (mocking simplified array)
        // Actually typical is: GPIOA, GPIOB... are peripherals.
        // Inside GPIO: CRL, CRH, IDR, ODR...
        // Let's model a register array: "P[%s]" dim=4, increment=4.

        let mut reg_info = svd_parser::svd::RegisterInfo::builder()
            .name("P[%s]".to_string())
            .address_offset(0x00);
        reg_info = reg_info.size(Some(32)).access(Some(SvdAccess::ReadWrite));
        let reg_info = reg_info.build(svd_parser::ValidateLevel::Disabled).unwrap();

        let dim = svd_parser::svd::DimElement::builder()
            .dim(4)
            .dim_increment(4)
            .build(svd_parser::ValidateLevel::Disabled)
            .unwrap();

        let reg_array = Register::Array(reg_info, dim);

        p = p.registers(Some(vec![RegisterCluster::Register(reg_array)]));

        let peripheral = p.build(svd_parser::ValidateLevel::Disabled).unwrap();
        let peripheral_enum = Peripheral::Single(peripheral);
        let device = Device::builder()
            .name("STM32".to_string())
            .peripherals(vec![])
            .build(svd_parser::ValidateLevel::Disabled)
            .unwrap();

        let desc = process_peripheral(&device, &peripheral_enum).unwrap();

        assert_eq!(desc.registers.len(), 4);
        assert_eq!(desc.registers[0].id, "P0"); // Default %s replacement
        assert_eq!(desc.registers[0].address_offset, 0x00);
        assert_eq!(desc.registers[1].id, "P1");
        assert_eq!(desc.registers[1].address_offset, 0x04);
        assert_eq!(desc.registers[3].id, "P3");
        assert_eq!(desc.registers[3].address_offset, 0x0C);
    }
    #[test]
    fn test_round_trip() {
        let mut p = PeripheralInfo::builder()
            .name("RT_PERIPH".to_string())
            .base_address(0x5000_0000);
        let mut reg = make_register("REG1", 0x00);
        // Add a field to ensure deep structure works
        let mut field = FieldInfo::builder().name("F1".to_string());
        field = field.bit_range(BitRange {
            offset: 0,
            width: 8,
            range_type: BitRangeType::BitRange,
        });
        if let Register::Single(ref mut info) = reg {
            info.fields = Some(vec![Field::Single(
                field.build(svd_parser::ValidateLevel::Disabled).unwrap(),
            )]);
        }
        p = p.registers(Some(vec![RegisterCluster::Register(reg)]));

        let peripheral = p.build(svd_parser::ValidateLevel::Disabled).unwrap();
        let peripheral_enum = Peripheral::Single(peripheral);
        let device = Device::builder()
            .name("STM32".to_string())
            .peripherals(vec![])
            .build(svd_parser::ValidateLevel::Disabled)
            .unwrap();

        let desc = process_peripheral(&device, &peripheral_enum).unwrap();

        // Serialize
        let yaml = serde_yaml::to_string(&desc).unwrap();

        // Deserialize back
        let deserialized: PeripheralDescriptor = serde_yaml::from_str(&yaml).unwrap();

        // Verify equality
        // PeripheralDescriptor derives PartialEq (checked in config crate? assuming yes or I check fields)
        // labwired_config doesn't explicitly derive PartialEq in my memory (viewed file was lib.rs but didn't check derive).
        // Let's assert on fields to be safe or check if I can derive it.
        // Actually, checking standard derive.

        assert_eq!(desc.peripheral, deserialized.peripheral);
        assert_eq!(desc.registers.len(), deserialized.registers.len());
        assert_eq!(desc.registers[0].id, deserialized.registers[0].id);
        assert_eq!(
            desc.registers[0].fields[0].name,
            deserialized.registers[0].fields[0].name
        );
    }
}
