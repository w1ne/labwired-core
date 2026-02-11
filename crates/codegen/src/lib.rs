use anyhow::Result;
use labwired_ir::{IrField, IrPeripheral, IrRegister};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

pub struct PeripheralGenerator;

impl PeripheralGenerator {
    pub fn generate(peripheral: &IrPeripheral) -> Result<String> {
        let original_name = &peripheral.name;
        let mut name = format_ident!("{}", original_name);

        // Check for collision with registers
        if peripheral
            .registers
            .iter()
            .any(|r| r.name == *original_name)
        {
            name = format_ident!("{}_PERIPHERAL", original_name);
        }

        let base_address = peripheral.base_address;
        let description = peripheral
            .description
            .as_deref()
            .unwrap_or("No description");

        let mut registers_code = Vec::new();
        let mut seen_registers = std::collections::HashSet::new();

        for reg in &peripheral.registers {
            if seen_registers.insert(reg.name.clone()) {
                registers_code.push(Self::generate_register(reg)?);
            }
        }

        let mod_name = format_ident!("{}", peripheral.name.to_lowercase());

        let expanded = quote! {
            pub mod #mod_name {
                #[doc = #description]
                pub struct #name;

                impl #name {
                    pub const BASE_ADDR: u64 = #base_address;
                }

                #(#registers_code)*
            }
        };

        Ok(expanded.to_string())
    }

    fn generate_register(reg: &IrRegister) -> Result<TokenStream> {
        let struct_name = format_ident!("{}", reg.name);
        let reset_value = reg.reset_value;
        let description = reg.description.as_deref().unwrap_or("No description");

        let mut field_methods = Vec::new();
        for field in &reg.fields {
            field_methods.push(Self::generate_field(field)?);
        }

        let expanded = quote! {
            #[doc = #description]
            pub struct #struct_name(u32);

            impl #struct_name {
                pub const RESET_VALUE: u32 = #reset_value as u32;

                pub fn new() -> Self {
                    Self(Self::RESET_VALUE)
                }

                pub fn reset(&mut self) {
                    self.0 = Self::RESET_VALUE;
                }

                #(#field_methods)*

                pub fn raw(&self) -> u32 {
                    self.0
                }

                pub fn set_raw(&mut self, value: u32) {
                    self.0 = value;
                }
            }
        };

        Ok(expanded)
    }

    fn sanitize_name(name: &str) -> String {
        let keywords = [
            "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
            "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
            "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
            "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become",
            "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual",
            "yield", "try",
        ];
        if keywords.contains(&name) {
            format!("r#{}", name)
        } else {
            name.to_string()
        }
    }

    fn generate_field(field: &IrField) -> Result<TokenStream> {
        let name_str = field.name.to_lowercase();
        // Avoid collision with register methods
        let name_str = if ["reset", "new", "raw", "set_raw"].contains(&name_str.as_str()) {
            format!("{}_field", name_str)
        } else {
            name_str
        };

        let name = format_ident!("{}", Self::sanitize_name(&name_str));
        let set_name = format_ident!("set_{}", name_str);
        let bit_offset = field.bit_offset;
        let bit_width = field.bit_width;
        let mask = if bit_width == 32 {
            0xffffffffu32
        } else {
            (1u32 << bit_width) - 1
        };
        let description = field.description.as_deref().unwrap_or("No description");

        // Generate getter (always)
        let getter = quote! {
            #[doc = #description]
            pub fn #name(&self) -> u32 {
                (self.0 >> #bit_offset) & #mask
            }
        };

        // Generate setter only if access allows write
        let setter = if matches!(
            field.access,
            None | Some(labwired_ir::IrAccess::ReadWrite)
                | Some(labwired_ir::IrAccess::WriteOnly)
                | Some(labwired_ir::IrAccess::Write1ToClear)
        ) {
            quote! {
                #[doc = #description]
                pub fn #set_name(&mut self, value: u32) {
                    let value_masked = value & #mask;
                    self.0 &= !(#mask << #bit_offset);
                    self.0 |= value_masked << #bit_offset;
                }
            }
        } else {
            quote! {}
        };

        Ok(quote! {
            #getter
            #setter
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_ir::{IrAccess, IrField, IrPeripheral, IrRegister};

    #[test]
    fn test_codegen_basic() {
        let peripheral = IrPeripheral {
            name: "UART".to_string(),
            base_address: 0x4000_1000,
            description: Some("Test UART".to_string()),
            registers: vec![IrRegister {
                name: "CR1".to_string(),
                offset: 0,
                size: 32,
                access: IrAccess::ReadWrite,
                reset_value: 0,
                description: Some("Control Register 1".to_string()),
                fields: vec![
                    IrField {
                        name: "UE".to_string(),
                        bit_offset: 0,
                        bit_width: 1,
                        access: None,
                        description: Some("USART enable".to_string()),
                    },
                    IrField {
                        name: "M".to_string(),
                        bit_offset: 12,
                        bit_width: 1,
                        access: None,
                        description: Some("Word length".to_string()),
                    },
                ],
            }],
            interrupts: vec![],
        };

        let result = PeripheralGenerator::generate(&peripheral).unwrap();
        println!("{}", result);
        assert!(result.contains("pub mod uart {"));
        assert!(result.contains("struct UART"));
        assert!(result.contains("struct CR1"));
        assert!(result.contains("const RESET_VALUE"));
        assert!(result.contains("0u32"));
        assert!(result.contains("pub fn reset"));
        assert!(result.contains("fn ue"));
        assert!(result.contains("fn set_ue"));
    }

    #[test]
    fn test_codegen_keywords() {
        let peripheral = IrPeripheral {
            name: "TEST".to_string(),
            base_address: 0x4000_0000,
            description: None,
            registers: vec![IrRegister {
                name: "REG".to_string(),
                offset: 0,
                size: 32,
                access: IrAccess::ReadWrite,
                reset_value: 0,
                description: None,
                fields: vec![
                    IrField {
                        name: "match".to_string(),
                        bit_offset: 0,
                        bit_width: 1,
                        access: None,
                        description: None,
                    },
                    IrField {
                        name: "type".to_string(),
                        bit_offset: 1,
                        bit_width: 1,
                        access: None,
                        description: None,
                    },
                    IrField {
                        name: "pub".to_string(),
                        bit_offset: 2,
                        bit_width: 1,
                        access: None,
                        description: None,
                    },
                ],
            }],
            interrupts: vec![],
        };

        let result = PeripheralGenerator::generate(&peripheral).unwrap();
        assert!(result.contains("fn r#match"));
        assert!(result.contains("fn set_match")); // Setters are usually safe as set_match isn't a keyword
        assert!(result.contains("fn r#type"));
        assert!(result.contains("fn r#pub"));
    }

    #[test]
    fn test_codegen_reserved_names() {
        let peripheral = IrPeripheral {
            name: "TEST".to_string(),
            base_address: 0x4000_0000,
            description: None,
            registers: vec![IrRegister {
                name: "REG".to_string(),
                offset: 0,
                size: 32,
                access: IrAccess::ReadWrite,
                reset_value: 0,
                description: None,
                fields: vec![
                    IrField {
                        name: "reset".to_string(),
                        bit_offset: 0,
                        bit_width: 1,
                        access: None,
                        description: None,
                    },
                    IrField {
                        name: "new".to_string(),
                        bit_offset: 1,
                        bit_width: 1,
                        access: None,
                        description: None,
                    },
                    IrField {
                        name: "raw".to_string(),
                        bit_offset: 2,
                        bit_width: 1,
                        access: None,
                        description: None,
                    },
                ],
            }],
            interrupts: vec![],
        };

        let result = PeripheralGenerator::generate(&peripheral).unwrap();
        // Should trigger the collision avoidance logic
        assert!(result.contains("fn reset_field"));
        assert!(result.contains("fn set_reset")); // Setters don't collide with reset()
        assert!(result.contains("fn new_field"));
        assert!(result.contains("fn raw_field"));
    }

    #[test]
    fn test_codegen_peripheral_collision() {
        let peripheral = IrPeripheral {
            name: "UART".to_string(),
            base_address: 0x4000_0000,
            description: None,
            registers: vec![IrRegister {
                name: "UART".to_string(),
                offset: 0,
                size: 32,
                access: IrAccess::ReadWrite,
                reset_value: 0,
                description: None,
                fields: vec![],
            }],
            interrupts: vec![],
        };

        let result = PeripheralGenerator::generate(&peripheral).unwrap();
        // Inner struct should be renamed to avoid collision with module/register names if they overlap
        // logic: if peripheral.registers.any(|r| r.name == peripheral.name) -> name = name_PERIPHERAL
        assert!(result.contains("struct UART_PERIPHERAL"));
        assert!(result.contains("struct UART")); // The register struct
    }
}
