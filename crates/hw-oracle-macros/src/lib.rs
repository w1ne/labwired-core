//! #[hw_oracle_test] macro. Plan 1 ships a passthrough placeholder;
//! real expansion lands in Task J3.

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn hw_oracle_test(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Placeholder: behave as #[test] for now. Expanded in Task J3.
    format!("#[test]\n{}", item.to_string()).parse().unwrap()
}
