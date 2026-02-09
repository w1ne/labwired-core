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
    _device: &Device,
    peripheral: &Peripheral,
) -> Result<PeripheralDescriptor> {
    let mut registers = Vec::new();
    let mut interrupts = HashMap::new();

    if let Some(children) = &peripheral.registers {
        for cluster in children {
            process_register_cluster(cluster, &mut registers, 0)?;
        }
    }

    // Sort registers by offset for cleaner output
    registers.sort_by_key(|r| r.address_offset);

    for interrupt in &peripheral.interrupt {
        interrupts.insert(interrupt.name.clone(), interrupt.value);
    }

    Ok(PeripheralDescriptor {
        peripheral: peripheral.name.clone(),
        version: "0.1.0".to_string(),
        registers,
        interrupts: if interrupts.is_empty() {
            None
        } else {
            Some(interrupts)
        },
    })
}

fn process_register_cluster(
    cluster: &RegisterCluster,
    registers: &mut Vec<RegisterDescriptor>,
    parent_offset: u64,
) -> Result<()> {
    match cluster {
        RegisterCluster::Register(reg) => {
            expand_register(reg, registers, parent_offset)?;
        }
        RegisterCluster::Cluster(cluster) => {
            expand_cluster(cluster, registers, parent_offset)?;
        }
    }
    Ok(())
}

fn expand_register(
    reg_array: &Register,
    registers: &mut Vec<RegisterDescriptor>,
    parent_offset: u64,
) -> Result<()> {
    match reg_array {
        Register::Single(reg) => {
            if let Some(desc) = convert_register(reg, parent_offset)? {
                registers.push(desc);
            }
        }
        Register::Array(reg, dim) => {
            for i in 0..dim.dim {
                let name = replace_dim_name(&reg.name, i, dim);
                let offset = parent_offset
                    + (reg.address_offset as u64)
                    + (i as u64 * dim.dim_increment as u64);

                // Clone and patch register info for specific instance
                // Note: We can't easily modify 'reg' effectively without cloning heavy structures
                // But convert_register only looks at properties.
                // Or we can pass name and offset override to convert_register.
                // Let's modify convert_register signature to accept overrides or the resolved items.

                if let Some(mut desc) = convert_register(reg, 0)? {
                    // 0 because we calculate full offset manually
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
) -> Result<()> {
    match cluster_array {
        svd_parser::svd::Cluster::Single(cluster) => {
            let current_offset = parent_offset + cluster.address_offset as u64;
            for child in &cluster.children {
                process_register_cluster(child, registers, current_offset)?;
            }
        }
        svd_parser::svd::Cluster::Array(cluster, dim) => {
            for i in 0..dim.dim {
                // let name = replace_dim_name(&cluster.name, i, dim); // Cluster name isn't directly used in flat list, but might be needed for debug or nested naming?
                // For flattened registers, we just care about the offset shift.
                // But wait, flattened registers usually inherit cluster name prefix?
                // SVD spec: "The name of the register is the name of the cluster followed by the name of the register".
                // Actually usually tools join them: "CLUSTER_REG".
                // But svd-parser doesn't join them?
                // LabWired has flat register list. If we don't prefix, we might have collisions.
                // For MVP, assuming SVD register names are unique or fully qualified?
                // Many SVDs use "ClusterName_RegName" for registers inside.
                // Some use nested name.
                // Let's implement name prefixing to be safe?
                // Or rely on SVD being well-formed.
                // The SVD spec says register names must be unique within a peripheral.
                // If they are in a cluster, the cluster namespace usually matters.
                // But let's stick to offset calculation first.

                let current_offset = parent_offset
                    + (cluster.address_offset as u64)
                    + (i as u64 * dim.dim_increment as u64);
                for child in &cluster.children {
                    process_register_cluster(child, registers, current_offset)?;
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
) -> Result<Option<RegisterDescriptor>> {
    let offset = extra_offset + reg.address_offset as u64;

    let size = reg.properties.size.unwrap_or(32) as u8;
    let access = match reg.properties.access {
        Some(SvdAccess::ReadWrite) => Access::ReadWrite,
        Some(SvdAccess::ReadOnly) => Access::ReadOnly,
        Some(SvdAccess::WriteOnly) => Access::WriteOnly,
        None => Access::ReadWrite,
        _ => Access::ReadWrite,
    };

    let reset_value = reg.properties.reset_value.unwrap_or(0) as u32;

    let mut fields = Vec::new();
    if let Some(svd_fields) = &reg.fields {
        for field in svd_fields {
            // Fields can also be arrays... expanding them is needed for completeness.
            // For now, let's treat field arrays as single fields (often just bit repetitions).
            // Or expand them.
            // Let's assume single fields for this step to reduce complexity, as they are inside register.
            match field {
                Field::Single(f) => fields.push(convert_field(f)?),
                Field::Array(f, dim) => {
                    for i in 0..dim.dim {
                        let mut f_clone = f.clone();
                        f_clone.name = replace_dim_name(&f.name, i, dim);
                        // bit-offset shift?
                        // Field bit range is static in struct...
                        // SVD spec: "The bit position of the field is ... plus dimIndex * dimIncrement".
                        let shift = i * dim.dim_increment;
                        let _width = f.bit_range.width;
                        let original_lsb = f.bit_range.offset;
                        f_clone.bit_range.offset = original_lsb + shift;

                        fields.push(convert_field(&f_clone)?);
                    }
                }
            }
        }
    }

    Ok(Some(RegisterDescriptor {
        id: reg.name.clone(),
        address_offset: offset,
        size,
        access,
        reset_value,
        fields,
        side_effects: None,
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
            let desc = convert_register(&reg, 0).unwrap().unwrap();
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
