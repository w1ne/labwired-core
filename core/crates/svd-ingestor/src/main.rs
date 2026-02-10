use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use svd_ingestor::{process_peripheral, save_descriptor};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Convert SVD files to LabWired Peripheral Descriptors"
)]
struct Args {
    /// Input SVD file
    #[arg(short, long)]
    input: PathBuf,

    /// Output directory for generated YAML files
    #[arg(short, long)]
    output_dir: PathBuf,

    /// Filter specific peripherals (comma separated)
    #[arg(long)]
    filter: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let xml = fs::read_to_string(&args.input).context("Failed to read SVD file")?;
    let device = svd_parser::parse(&xml).context("Failed to parse SVD XML")?;

    fs::create_dir_all(&args.output_dir).context("Failed to create output directory")?;

    let filter_list: Option<Vec<String>> = args
        .filter
        .map(|s| s.split(',').map(|s| s.trim().to_string()).collect());

    for peripheral in &device.peripherals {
        if let Some(ref filters) = filter_list {
            if !filters.contains(&peripheral.name) {
                continue;
            }
        }

        println!("Processing peripheral: {}", peripheral.name);
        match process_peripheral(&device, peripheral) {
            Ok(descriptor) => {
                save_descriptor(&descriptor, &args.output_dir)?;
            }
            Err(e) => {
                eprintln!("Failed to process peripheral {}: {}", peripheral.name, e);
            }
        }
    }

    Ok(())
}
