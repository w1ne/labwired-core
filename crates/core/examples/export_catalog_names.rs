use labwired_core::session::catalog::Catalog;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", serde_json::to_string(&Catalog::discover().chip_names())?);
    Ok(())
}
