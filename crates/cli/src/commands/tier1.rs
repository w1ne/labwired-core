// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired tier1-matrix` Tier-1 conformance matrix.

use crate::*;

pub(crate) fn run_tier1_matrix(args: Tier1MatrixArgs) -> ExitCode {
    let self_bin = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: cannot resolve current executable: {e}");
            return ExitCode::FAILURE;
        }
    };
    match labwired_cli::tier1::run_all(&self_bin) {
        Ok((mut matrix, skipped)) => {
            for chip in &skipped {
                eprintln!("SKIP: {chip} (fixture not present)");
            }
            // --run-url given but nothing was actually exercised → vacuous green
            // is not permitted; fail loudly so CI notices the misconfiguration.
            // (Skipped targets still emit unrecorded rows, so key on the count
            // of EXERCISED chips, not on matrix emptiness.)
            if args.run_url.is_some() && matrix.0.len() == skipped.len() {
                eprintln!("error: --run-url given but no fixtures were exercised");
                return ExitCode::FAILURE;
            }
            if let Some(url) = &args.run_url {
                use labwired_cli::tier1::CellStatus;
                for row in matrix.0.values_mut() {
                    for cell in row.values_mut() {
                        if cell.status != CellStatus::Unrecorded && cell.status != CellStatus::Na {
                            cell.run_url = Some(url.clone());
                        }
                    }
                }
            }
            // Text grid for humans.
            for (chip, row) in &matrix.0 {
                let cells: Vec<String> = row
                    .iter()
                    .map(|(class, cell)| format!("{class}={}", cell.status.as_str()))
                    .collect();
                println!("{chip}: {}", cells.join(" "));
            }
            if let Some(out) = &args.json_out {
                let json = match serde_json::to_string_pretty(&matrix) {
                    Ok(j) => j,
                    Err(e) => {
                        eprintln!("error: failed to serialize tier1 matrix: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                if let Err(e) = std::fs::write(out, json.as_bytes()) {
                    eprintln!("error: failed to write {}: {e}", out.display());
                    return ExitCode::FAILURE;
                }
                eprintln!("wrote {}", out.display());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("tier1-matrix failed: {e}");
            ExitCode::FAILURE
        }
    }
}
