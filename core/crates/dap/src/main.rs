// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_dap::server::DapServer;
use std::io;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let _stdin = io::stdin();
    let _stdout = io::stdout();

    let server = DapServer::new();
    server.run(io::stdin(), io::stdout())?;

    Ok(())
}
