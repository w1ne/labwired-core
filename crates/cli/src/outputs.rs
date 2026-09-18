// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::*;

#[allow(clippy::too_many_arguments, clippy::if_same_then_else)]
pub(crate) fn write_outputs<C: labwired_core::Cpu>(
    args: &TestArgs,
    // The run's ONE verdict, not a status string. Taking `&str` here meant a
    // caller could invent a status that disagreed with the exit code it went on
    // to return; that is the drift `crate::verdict` exists to make
    // unrepresentable, so the artifact writer is handed the verdict itself.
    verdict: crate::verdict::Verdict,
    steps_executed: u64,
    metrics: &labwired_core::metrics::PerformanceMetrics,
    stop_reason: StopReason,
    stop_reason_details: StopReasonDetails,
    // Set only when the firmware ended its own run through `simctl`.
    firmware_exit_code: Option<u32>,
    limits: TestLimits,
    assertions: Vec<AssertionResult>,
    firmware_bytes: &[u8],
    uart_tx: &Arc<Mutex<Vec<u8>>>,
    rtt_tx: &Arc<Mutex<Vec<u8>>>,
    rtt_status: Option<labwired_core::peripherals::segger_rtt::RttStatus>,
    cpu: &C,
    firmware_path: &Path,
    system_path: Option<&PathBuf>,
    duration: std::time::Duration,
    trace_observer: &Option<Arc<labwired_core::trace::TraceObserver>>,
    coverage_observer: &Option<Arc<labwired_core::pc_coverage::PcCoverageObserver>>,
    fault_evidence: &[labwired_cli::faults::FaultEvidence],
    inspect: Option<labwired_core::inspect::MachineInspect>,
    logic_edges: Option<labwired_core::logic_capture::LogicEdgesResult>,
    stimuli: Vec<StimulusOutcome>,
    footprint: Option<artifacts::FootprintReport>,
    memory: Option<labwired_core::stack_paint::MainStackReport>,
    metrics_block: Option<artifacts::ExecutionMetrics>,
) {
    let status = verdict.status();

    let mut hasher = Sha256::new();
    hasher.update(firmware_bytes);
    let firmware_hash = format!("{:x}", hasher.finalize());

    // Drain the coverage-gap log for THIS run. `write_outputs` is called
    // synchronously at the tail of `execute_test_loop`, on the very thread that
    // ran the sim loop, so this reads the same thread-local the `record_*` calls
    // populated. `take()` resets it, so it must run exactly once per run — this
    // is the sole call site on the run path.
    let fidelity = labwired_core::fidelity::take().to_gaps();

    // Silent-path census (measurement only). Compiled to an empty function
    // unless `--features silent-census`, and even then writes nothing unless
    // LABWIRED_CENSUS_OUT names a path. Sits here because `write_outputs` is
    // the sole call site on the run path, reached synchronously at the tail of
    // `execute_test_loop` on the thread that ran the sim.
    labwired_core::census::dump_if_requested();

    // Derive the top-level `message` from the stimulus block rather than taking
    // it as a second parameter, so the human sentence and the structured
    // evidence cannot drift apart — one source of truth. A rejected stimulus is
    // fatal, so a reader who only ever looks at `status` + `message` still
    // cannot miss it.
    let rejected: Vec<String> = stimuli
        .iter()
        .filter(|o| o.is_rejected())
        .map(|o| o.describe())
        .collect();
    let message = (!rejected.is_empty()).then(|| {
        format!(
            "{} stimulus/stimuli could not be applied, so the run proved nothing about them: {}",
            rejected.len(),
            rejected.join("; ")
        )
    });

    let assertions_for_junit = assertions.clone();
    let result = TestResult {
        result_schema_version: RESULT_SCHEMA_VERSION.to_string(),
        status: status.to_string(),
        steps_executed,
        cycles: metrics.get_cycles(),
        instructions: metrics.get_instructions(),
        stop_reason,
        stop_reason_details: stop_reason_details.clone(),
        firmware_exit_code,
        limits: limits.clone(),
        message,
        assertions,
        cpu_state: Some(cpu.snapshot()),
        firmware_hash,
        config: TestConfig {
            firmware: firmware_path.to_path_buf(),
            system: system_path.cloned(),
            script: args.script.clone(),
        },
        inspect,
        fidelity,
        logic_edges,
        stimuli,
        footprint,
        memory,
        metrics: metrics_block,
        rtt: rtt_status,
    };

    if let Some(output_dir) = &args.output_dir {
        if let Err(e) = std::fs::create_dir_all(output_dir) {
            error!("Failed to create output directory {:?}: {}", output_dir, e);
        } else {
            // result.json
            let result_path = output_dir.join("result.json");
            match std::fs::File::create(&result_path) {
                Ok(f) => {
                    if let Err(e) = serde_json::to_writer_pretty(f, &result) {
                        error!("Failed to write result.json: {}", e);
                    }
                }
                Err(e) => error!("Failed to create result.json: {}", e),
            }

            // trace.json
            if let Some(obs) = trace_observer {
                let trace_path = output_dir.join("trace.json");
                let traces = obs.take_traces();
                match std::fs::File::create(&trace_path) {
                    Ok(f) => {
                        if let Err(e) = serde_json::to_writer_pretty(f, &traces) {
                            error!("Failed to write trace.json: {}", e);
                        }
                    }
                    Err(e) => error!("Failed to create trace.json: {}", e),
                }
            }

            // fault-evidence.json (per-fault verdicts; also folded into the manifest)
            if !fault_evidence.is_empty() {
                let fault_path = output_dir.join("fault-evidence.json");
                match std::fs::File::create(&fault_path) {
                    Ok(f) => {
                        if let Err(e) = serde_json::to_writer_pretty(f, fault_evidence) {
                            error!("Failed to write fault-evidence.json: {}", e);
                        }
                    }
                    Err(e) => error!("Failed to create fault-evidence.json: {}", e),
                }
            }

            // coverage.info (LCOV) + coverage.json
            let mut coverage_summary: Option<labwired_cli::manifest::CoverageSummary> = None;
            if let Some(cov) = coverage_observer {
                match labwired_loader::SymbolProvider::new(firmware_path) {
                    Ok(symbols) => {
                        let mut report = labwired_cli::pc_coverage_report::CoverageReport::build(
                            symbols.statement_rows(),
                            |addr| cov.was_executed(addr as u32),
                        );
                        // Resolve each observed branch site to its source line.
                        let branch_cov = cov
                            .branch_sites()
                            .into_iter()
                            .filter_map(|(src, counts)| {
                                symbols.lookup(src as u64).and_then(|loc| {
                                    loc.line.map(|line| {
                                        // statement_rows uses the line-program
                                        // file basename; lookup() returns the
                                        // full path. Normalise to the basename
                                        // so branches attach to the right SF.
                                        let file = loc
                                            .file
                                            .rsplit('/')
                                            .next()
                                            .unwrap_or(&loc.file)
                                            .to_string();
                                        labwired_cli::pc_coverage_report::BranchCoverage {
                                            file,
                                            line,
                                            taken: counts.taken,
                                            not_taken: counts.not_taken,
                                        }
                                    })
                                })
                            })
                            .collect();
                        report.set_branches(branch_cov);
                        let info_path = output_dir.join("coverage.info");
                        if let Err(e) = std::fs::write(&info_path, report.to_lcov()) {
                            error!("Failed to write coverage.info: {}", e);
                        }
                        let cov_json_path = output_dir.join("coverage.json");
                        match std::fs::File::create(&cov_json_path) {
                            Ok(f) => {
                                if let Err(e) = serde_json::to_writer_pretty(f, &report) {
                                    error!("Failed to write coverage.json: {}", e);
                                }
                            }
                            Err(e) => error!("Failed to create coverage.json: {}", e),
                        }
                        info!(
                            "Coverage: {}/{} statements ({:.1}%), {}/{} branches ({:.1}%)",
                            report.covered_statements,
                            report.total_statements,
                            report.statement_percent(),
                            report.covered_branches,
                            report.total_branches,
                            report.branch_percent()
                        );
                        coverage_summary = Some(labwired_cli::manifest::CoverageSummary {
                            statements_total: report.total_statements,
                            statements_covered: report.covered_statements,
                            branches_total: report.total_branches,
                            branches_covered: report.covered_branches,
                        });
                    }
                    Err(e) => error!("Failed to load symbols for coverage: {}", e),
                }
            }

            // run-manifest.json (signable, reproducible)
            if args.run_manifest {
                use labwired_cli::manifest;
                // Use the file basename, not the absolute path, so the digest
                // depends only on file contents and is reproducible across
                // machines with different checkout locations.
                let basename = |p: &Path| -> String {
                    p.file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.display().to_string())
                };
                let hash_file = |p: &Path| -> manifest::HashedFile {
                    let sha256 = std::fs::read(p)
                        .map(|b| manifest::sha256_hex(&b))
                        .unwrap_or_default();
                    manifest::HashedFile {
                        path: basename(p),
                        sha256,
                    }
                };
                let mut configs = vec![hash_file(&args.script)];
                if let Some(sys) = system_path {
                    configs.push(hash_file(sys));
                }
                // Stamp honestly: `any_noise_enabled` matches kit config keys
                // and declarative noise inputs. Seeded noise is bit-identical
                // across runs, but it is not the absence of variation.
                let nondeterminism = system_path
                    .and_then(|sys| std::fs::read_to_string(sys).ok())
                    .filter(|yaml| manifest::any_noise_enabled(yaml))
                    .map(|_| "seeded(sensor-noise)".to_string())
                    .unwrap_or_else(|| "none".to_string());
                let mut man = manifest::RunManifest {
                    manifest_schema_version: manifest::MANIFEST_SCHEMA_VERSION.to_string(),
                    engine_version: env!("CARGO_PKG_VERSION").to_string(),
                    seed: 0,
                    nondeterminism,
                    firmware: manifest::HashedFile {
                        path: basename(firmware_path),
                        sha256: result.firmware_hash.clone(),
                    },
                    configs,
                    results: manifest::ManifestResults {
                        status: status.to_string(),
                        stop_reason: format!("{:?}", result.stop_reason),
                        steps_executed: result.steps_executed,
                        cycles: result.cycles,
                        instructions: result.instructions,
                        assertions: assertions_for_junit
                            .iter()
                            .map(|a| manifest::AssertionOutcome {
                                assertion: format!("{:?}", a.assertion),
                                passed: a.passed,
                            })
                            .collect(),
                        cpu_state_digest: manifest::digest_value(&cpu.snapshot()),
                    },
                    coverage: coverage_summary.clone(),
                    fault_injections: fault_evidence.to_vec(),
                    digest: String::new(),
                };
                man.finalize_digest();
                let manifest_path = output_dir.join("run-manifest.json");
                match std::fs::File::create(&manifest_path) {
                    Ok(f) => {
                        if let Err(e) = serde_json::to_writer_pretty(f, &man) {
                            error!("Failed to write run-manifest.json: {}", e);
                        }
                    }
                    Err(e) => error!("Failed to create run-manifest.json: {}", e),
                }
                info!("Run manifest digest: {}", man.digest);
            }

            // result.json handles cpu generically now
            let snapshot_path = output_dir.join("snapshot.json");
            let snapshot = Snapshot::Standard {
                cpu: cpu.snapshot(),
                steps_executed,
                cycles: result.cycles,
                instructions: result.instructions,
                stop_reason: result.stop_reason.clone(),
                stop_reason_details: result.stop_reason_details.clone(),
                limits: result.limits.clone(),
                firmware_hash: result.firmware_hash.clone(),
                config: TestConfig {
                    firmware: result.config.firmware.clone(),
                    system: result.config.system.clone(),
                    script: result.config.script.clone(),
                },
            };
            match std::fs::File::create(&snapshot_path) {
                Ok(f) => {
                    if let Err(e) = serde_json::to_writer_pretty(f, &snapshot) {
                        error!("Failed to write snapshot.json: {}", e);
                    }
                }
                Err(e) => error!("Failed to create snapshot.json: {}", e),
            }

            // uart.log
            let uart_path = output_dir.join("uart.log");
            let bytes = uart_tx.lock().map(|g| g.clone()).unwrap_or_default();
            if let Err(e) = std::fs::write(&uart_path, bytes) {
                error!("Failed to write uart.log: {}", e);
            }

            // rtt.log — the dedicated RTT stream, never spliced with UART.
            // Written like uart.log on every output-dir run; it stays empty
            // when RTT was not enabled, and result.json's `rtt` block is the
            // enable/status signal (so "enabled and silent" is readable).
            let rtt_path = output_dir.join("rtt.log");
            let bytes = rtt_tx.lock().map(|g| g.clone()).unwrap_or_default();
            if let Err(e) = std::fs::write(&rtt_path, bytes) {
                error!("Failed to write rtt.log: {}", e);
            }

            // junit.xml
            let junit_path = output_dir.join("junit.xml");
            if let Err(e) = write_junit_xml(&junit_path, status, duration, &result) {
                error!("Failed to write junit.xml: {}", e);
            }
        }
    }

    if let Some(junit_path) = &args.junit {
        if let Some(parent) = junit_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = write_junit_xml(junit_path, status, duration, &result) {
            error!("Failed to write JUnit report {:?}: {}", junit_path, e);
        }
    }
}

pub(crate) fn write_config_error_outputs(
    args: &TestArgs,
    firmware_path: Option<&PathBuf>,
    system_path: Option<&PathBuf>,
    firmware_bytes: Option<&[u8]>,
    limits: Option<&TestLimits>,
    message: String,
) {
    // Best-effort: the caller requests artifacts, but directory creation / writes may fail.
    let firmware_hash = match firmware_bytes {
        Some(bytes) => {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            format!("{:x}", hasher.finalize())
        }
        None => String::new(),
    };

    let resolved_limits = limits.cloned().unwrap_or(TestLimits {
        max_steps: 0,
        max_cycles: None,
        max_uart_bytes: None,
        no_progress_steps: None,
        wall_time_ms: None,
        max_vcd_bytes: None,
        stop_when_assertions_pass: false,
        stop_when_assertions_pass_settle_steps: 0,
        stop_when_assertions_pass_min_steps: 0,
    });

    let stop_reason = StopReason::ConfigError;
    let stop_reason_details = crate::report::build_stop_reason_details(
        &stop_reason,
        &resolved_limits,
        0,
        0,
        0,
        0,
        std::time::Duration::from_secs(0),
        0, // vcd_bytes
    );

    let result = TestResult {
        result_schema_version: RESULT_SCHEMA_VERSION.to_string(),
        status: "error".to_string(),
        steps_executed: 0,
        cycles: 0,
        instructions: 0,
        stop_reason,
        stop_reason_details: stop_reason_details.clone(),
        // A config error never ran firmware, so there is no verdict.
        firmware_exit_code: None,
        limits: resolved_limits.clone(),
        message: Some(message.clone()),
        assertions: vec![],
        cpu_state: None,
        firmware_hash,
        config: TestConfig {
            firmware: firmware_path.cloned().unwrap_or_default(),
            system: system_path.cloned(),
            script: args.script.clone(),
        },
        inspect: None,
        // Config error: the sim never ran, so there are no coverage gaps to report.
        fidelity: Vec::new(),
        // Nor any logic-analyzer edges — capture never armed.
        logic_edges: None,
        // Nor any stimulus outcomes: the run was rejected before a machine
        // existed, so no stimulus was ever attempted.
        stimuli: Vec::new(),
        // Config error: no firmware footprint or stack paint collected.
        footprint: None,
        memory: None,
        metrics: None,
        // Config error: no machine, so no RTT model and no diagnostics.
        rtt: None,
    };

    if let Some(output_dir) = &args.output_dir {
        if let Err(e) = std::fs::create_dir_all(output_dir) {
            error!("Failed to create output directory {:?}: {}", output_dir, e);
        } else {
            let result_path = output_dir.join("result.json");
            match std::fs::File::create(&result_path) {
                Ok(f) => {
                    if let Err(e) = serde_json::to_writer_pretty(f, &result) {
                        error!("Failed to write result.json: {}", e);
                    }
                }
                Err(e) => error!("Failed to create result.json: {}", e),
            }

            let snapshot_path = output_dir.join("snapshot.json");
            let snapshot = Snapshot::ConfigError {
                message: message.clone(),
                stop_reason_details: result.stop_reason_details.clone(),
                limits: result.limits.clone(),
                config: TestConfig {
                    firmware: result.config.firmware.clone(),
                    system: result.config.system.clone(),
                    script: result.config.script.clone(),
                },
            };
            match std::fs::File::create(&snapshot_path) {
                Ok(f) => {
                    if let Err(e) = serde_json::to_writer_pretty(f, &snapshot) {
                        error!("Failed to write snapshot.json: {}", e);
                    }
                }
                Err(e) => error!("Failed to create snapshot.json: {}", e),
            }

            let uart_path = output_dir.join("uart.log");
            if let Err(e) = std::fs::write(&uart_path, b"") {
                error!("Failed to write uart.log: {}", e);
            }

            let junit_path = output_dir.join("junit.xml");
            if let Err(e) = write_junit_xml(
                &junit_path,
                "error",
                std::time::Duration::from_secs(0),
                &result,
            ) {
                error!("Failed to write junit.xml: {}", e);
            }
        }
    }

    if let Some(junit_path) = &args.junit {
        if let Some(parent) = junit_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = write_junit_xml(
            junit_path,
            "error",
            std::time::Duration::from_secs(0),
            &result,
        ) {
            error!("Failed to write JUnit report {:?}: {}", junit_path, e);
        }
    }
}

pub(crate) fn resolve_script_path(script_path: &Path, value: &str) -> PathBuf {
    let p = PathBuf::from(value);
    if p.is_absolute() {
        return p;
    }
    script_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(p)
}

pub(crate) fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub(crate) fn write_junit_xml(
    path: &Path,
    status: &str,
    duration: std::time::Duration,
    outcome: &artifacts::TestOutcome,
) -> std::io::Result<()> {
    let stop_reason = &outcome.stop_reason;
    let assertions = &outcome.assertions[..];
    let firmware_hash = &outcome.firmware_hash;
    let config = &outcome.config;
    let message = outcome.message.as_deref();
    let steps_executed = outcome.steps_executed;
    let cycles = outcome.cycles;
    let instructions = outcome.instructions;
    let limits = &outcome.limits;
    let stop_reason_details = &outcome.stop_reason_details;

    let any_assertion_failed = assertions.iter().any(|a| !a.passed);
    let any_expected_stop_reason_matched = assertions
        .iter()
        .any(|a| matches!(a.assertion, TestAssertion::ExpectedStopReason(_)) && a.passed);
    let stop_requires_assertion = matches!(
        stop_reason,
        StopReason::WallTime | StopReason::MaxUartBytes | StopReason::NoProgress
    );

    let mut details = String::new();
    details.push_str(&format!(
        "result_schema_version={}\n",
        RESULT_SCHEMA_VERSION
    ));
    details.push_str(&format!("stop_reason={:?}\n", stop_reason));
    if let Some(msg) = message {
        details.push_str(&format!("message={}\n", msg));
    }
    details.push_str(&format!(
        "stop_reason_details.triggered_stop_condition={:?}\n",
        stop_reason_details.triggered_stop_condition
    ));
    if let Some(t) = &stop_reason_details.triggered_limit {
        details.push_str(&format!(
            "stop_reason_details.triggered_limit.{}={}\n",
            t.name, t.value
        ));
    }
    if let Some(o) = &stop_reason_details.observed {
        details.push_str(&format!(
            "stop_reason_details.observed.{}={}\n",
            o.name, o.value
        ));
    }
    details.push_str(&format!("steps_executed={}\n", steps_executed));
    details.push_str(&format!("cycles={}\n", cycles));
    details.push_str(&format!("instructions={}\n", instructions));
    details.push_str("limits:\n");
    details.push_str(&format!("  - max_steps={}\n", limits.max_steps));
    if let Some(v) = limits.max_cycles {
        details.push_str(&format!("  - max_cycles={}\n", v));
    }
    if let Some(v) = limits.max_uart_bytes {
        details.push_str(&format!("  - max_uart_bytes={}\n", v));
    }
    if let Some(v) = limits.no_progress_steps {
        details.push_str(&format!("  - no_progress_steps={}\n", v));
    }
    if let Some(v) = limits.wall_time_ms {
        details.push_str(&format!("  - wall_time_ms={}\n", v));
    }
    details.push_str(&format!("firmware_hash={}\n", firmware_hash));
    details.push_str(&format!("firmware={}\n", config.firmware.display()));
    if let Some(sys) = &config.system {
        details.push_str(&format!("system={}\n", sys.display()));
    }
    details.push_str(&format!("script={}\n", config.script.display()));
    if !assertions.is_empty() {
        details.push_str("assertions:\n");
        for a in assertions {
            details.push_str(&format!("  - {:?}: {}\n", a.assertion, a.passed));
        }
    }

    let time_secs = duration.as_secs_f64();

    let mut xml = String::new();
    xml.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    xml.push('\n');
    let mut tests: u64 = 0;
    let mut failures: u64 = 0;
    let mut errors: u64 = 0;

    let mut testcases = String::new();

    // A top-level "run" testcase captures non-assertion failures (e.g. stop condition without expected_stop_reason)
    // and runtime errors.
    tests += 1;
    testcases.push_str(&format!(
        "  <testcase classname=\"labwired\" name=\"run\" time=\"{:.6}\">\n",
        time_secs
    ));
    if status == "error" {
        let err_type = if *stop_reason == StopReason::ConfigError {
            "config error"
        } else {
            "runtime error"
        };
        errors += 1;
        testcases.push_str(&format!(
            "    <error message=\"{}\">{}</error>\n",
            xml_escape(err_type),
            xml_escape(&details)
        ));
    } else if status == "fail" && stop_requires_assertion && !any_expected_stop_reason_matched {
        failures += 1;
        testcases.push_str(&format!(
            "    <failure message=\"{}\">{}</failure>\n",
            xml_escape("stop condition requires expected_stop_reason assertion"),
            xml_escape(&details)
        ));
    } else if status == "fail" && (!any_assertion_failed) {
        failures += 1;
        testcases.push_str(&format!(
            "    <failure message=\"{}\">{}</failure>\n",
            xml_escape("failure"),
            xml_escape(&details)
        ));
    }
    testcases.push_str("  </testcase>\n");

    // One testcase per assertion so CI UIs show exactly which assertion failed.
    for (idx, a) in assertions.iter().enumerate() {
        tests += 1;
        let name = format!(
            "assertion {}: {}",
            idx + 1,
            assertion_short_name(&a.assertion)
        );
        testcases.push_str(&format!(
            "  <testcase classname=\"labwired\" name=\"{}\" time=\"0.000000\">\n",
            xml_escape(&name)
        ));
        if !a.passed {
            failures += 1;
            testcases.push_str(&format!(
                "    <failure message=\"assertion failed\">{}</failure>\n",
                xml_escape(&format!("{}\n\n{}", name, details))
            ));
        }
        testcases.push_str("  </testcase>\n");
    }

    xml.push_str(&format!(
        r#"<testsuite name="labwired" tests="{}" failures="{}" errors="{}" time="{:.6}">"#,
        tests, failures, errors, time_secs
    ));
    xml.push('\n');
    xml.push_str("  <properties>\n");
    xml.push_str(&format!(
        "    <property name=\"result_schema_version\" value=\"{}\"/>\n",
        xml_escape(RESULT_SCHEMA_VERSION)
    ));
    xml.push_str(&format!(
        "    <property name=\"stop_reason\" value=\"{}\"/>\n",
        xml_escape(&format!("{:?}", stop_reason))
    ));
    xml.push_str(&format!(
        "    <property name=\"firmware_hash\" value=\"{}\"/>\n",
        xml_escape(firmware_hash)
    ));
    xml.push_str("  </properties>\n");
    xml.push_str(&testcases);
    xml.push_str("</testsuite>\n");

    std::fs::write(path, xml)
}
