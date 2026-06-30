// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired coverage` register-modeling coverage report.

use crate::*;

pub(crate) fn run_coverage(args: CoverageArgs) -> ExitCode {
    if let Some(p) = &args.svd {
        std::env::set_var("LABWIRED_ESP32S3_SVD", p);
    }
    match coverage::run() {
        Some((matrix, text)) => {
            print!("{text}");
            if let Some(out) = &args.json_out {
                let json = serde_json::to_string_pretty(&matrix).expect("serialize matrix");
                std::fs::write(out, &json).expect("write json");
                eprintln!("wrote {}", out.display());
            }
            ExitCode::SUCCESS
        }
        None => {
            eprintln!(
                "error: ESP32-S3 SVD not found; set --svd or LABWIRED_ESP32S3_SVD, \
                 or install the espressif32 PlatformIO platform"
            );
            ExitCode::from(EXIT_CONFIG_ERROR)
        }
    }
}
