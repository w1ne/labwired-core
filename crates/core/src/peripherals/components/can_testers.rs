// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! PeripheralKits for host-side CAN tools formerly hand-wired in `from_config`.
//!
//! These are not SoC models — they are second-node injectors / log players that
//! hang off a named CAN controller (`connection:` → bxCAN/FDCAN id). Migrating
//! them off residual `from_config` arms keeps that match empty for product
//! devices and puts attach next to the types they construct.

use anyhow::anyhow;

use crate::bus::{CanDiagnosticTester, CanLogPlayer, CanUdsTester, SystemBus, UdsStep};
use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

fn require_can_connection(ctx: &AttachCtx<'_>, label: &str) -> anyhow::Result<()> {
    if ctx
        .bus
        .find_peripheral_index_by_name(ctx.connection())
        .is_none()
    {
        // Preserve legacy from_config wording so tests/logs still match
        // (`can-player 'p' connection 'nope' was not found`).
        return Err(anyhow!(
            "{label} '{}' connection '{}' was not found",
            ctx.device_id(),
            ctx.connection()
        ));
    }
    Ok(())
}

// ─── can-diagnostic-tester ───────────────────────────────────────────────────

pub struct CanDiagnosticTesterKit;
pub static CAN_DIAGNOSTIC_TESTER_KIT: CanDiagnosticTesterKit = CanDiagnosticTesterKit;

static CAN_DIAGNOSTIC_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "can-diagnostic-tester",
    label: "CAN diagnostic tester",
    summary: "One-shot single-frame UDS-style request injector on bxCAN/FDCAN.",
    detail: "Injects a single diagnostic request frame (default 0x7E0 / ReadDataByIdentifier) \
             once the connected CAN controller is up. Alias: uds-diagnostic-tester.",
    transport: Transport::Can,
    category: Category::Misc,
    config_keys: &[
        ConfigKey {
            name: "request_id",
            ty: ConfigType::Int,
            doc: "CAN id for the request (default 0x7E0).",
        },
        ConfigKey {
            name: "request_data",
            ty: ConfigType::Str,
            doc: "Hex/bytes payload (default 03 22 F1 90).",
        },
    ],
    labs: &[],
};

impl PeripheralKit for CanDiagnosticTesterKit {
    fn metadata(&self) -> &'static KitMetadata {
        &CAN_DIAGNOSTIC_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        require_can_connection(ctx, "CAN diagnostic tester")?;
        let request_id = SystemBus::yaml_u32(ctx.ext.config.get("request_id"), 0x7E0);
        let request_data = SystemBus::yaml_bytes(
            ctx.ext.config.get("request_data"),
            &[0x03, 0x22, 0xF1, 0x90],
        );
        ctx.bus.can_diagnostic_testers.push(CanDiagnosticTester {
            id: ctx.device_id().to_string(),
            connection: ctx.connection().to_string(),
            request_id,
            request_data,
            sent: false,
        });
        Ok(())
    }
}

// ─── uds-tester ──────────────────────────────────────────────────────────────

pub struct CanUdsTesterKit;
pub static CAN_UDS_TESTER_KIT: CanUdsTesterKit = CanUdsTesterKit;

static CAN_UDS_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "uds-tester",
    label: "UDS / ISO-TP tester",
    summary: "Stateful multi-frame UDS tester (SecurityAccess-class handshakes).",
    detail: "Second CAN node that drives ISO-TP First/Consecutive frames and \
             observes ECU responses via the public bxCAN/FDCAN inject API. \
             Optional `script:` steps; legacy first_frame/consecutive_frame still work.",
    transport: Transport::Can,
    category: Category::Misc,
    config_keys: &[
        ConfigKey {
            name: "request_id",
            ty: ConfigType::Int,
            doc: "Tester → ECU id (default 0x111).",
        },
        ConfigKey {
            name: "reply_id",
            ty: ConfigType::Int,
            doc: "ECU → tester id (default 0x222).",
        },
        ConfigKey {
            name: "first_frame",
            ty: ConfigType::Str,
            doc: "Legacy ISO-TP FirstFrame bytes when script is omitted.",
        },
        ConfigKey {
            name: "consecutive_frame",
            ty: ConfigType::Str,
            doc: "Legacy ISO-TP ConsecutiveFrame bytes when script is omitted.",
        },
        ConfigKey {
            name: "script",
            ty: ConfigType::Str,
            doc: "YAML list of {send, expect, expect_nrc} steps.",
        },
    ],
    labs: &[],
};

impl PeripheralKit for CanUdsTesterKit {
    fn metadata(&self) -> &'static KitMetadata {
        &CAN_UDS_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        require_can_connection(ctx, "UDS tester")?;
        let mut tester = CanUdsTester::new(ctx.device_id().to_string(), ctx.connection().to_string());
        tester.request_id =
            SystemBus::yaml_u32(ctx.ext.config.get("request_id"), CanUdsTester::DEFAULT_REQUEST_ID);
        tester.reply_id =
            SystemBus::yaml_u32(ctx.ext.config.get("reply_id"), CanUdsTester::DEFAULT_REPLY_ID);
        tester.first_frame = SystemBus::yaml_bytes(
            ctx.ext.config.get("first_frame"),
            &CanUdsTester::DEFAULT_FIRST_FRAME,
        );
        tester.consecutive_frame = SystemBus::yaml_bytes(
            ctx.ext.config.get("consecutive_frame"),
            &CanUdsTester::DEFAULT_CONSECUTIVE_FRAME,
        );
        tester.script = SystemBus::parse_script(ctx.ext.config.get("script"));
        // When no `script:` key is present, synthesize a single step from the
        // legacy first_frame / consecutive_frame fields.
        if !ctx.ext.config.contains_key("script") {
            let ff = &tester.first_frame;
            let pdu_len = if ff.len() >= 2 {
                (((ff[0] & 0x0F) as usize) << 8) | (ff[1] as usize)
            } else {
                0
            };
            if ctx.ext.config.contains_key("first_frame") && (ff.len() < 2 || pdu_len == 0) {
                tracing::warn!(
                    "[uds-tester] '{}': first_frame is too short or decodes pdu_len=0 \
                     — synthesized send will be empty",
                    ctx.device_id()
                );
            }
            let ff_payload: &[u8] = if ff.len() >= 2 { &ff[2..] } else { &[] };
            let cf_payload: &[u8] = if !tester.consecutive_frame.is_empty() {
                &tester.consecutive_frame[1..]
            } else {
                &[]
            };
            let raw: Vec<u8> = ff_payload
                .iter()
                .chain(cf_payload.iter())
                .copied()
                .take(pdu_len)
                .collect();
            if raw.is_empty() && ctx.ext.config.contains_key("first_frame") {
                tracing::warn!(
                    "[uds-tester] '{}': reassembled send payload is empty \
                     — check first_frame / consecutive_frame config",
                    ctx.device_id()
                );
            }
            tester.script = vec![UdsStep {
                send: raw,
                expect: vec![Some(0x06), Some(0x67)],
                expect_nrc: None,
            }];
        }
        ctx.bus.can_uds_testers.push(tester);
        Ok(())
    }
}

// ─── can-player ──────────────────────────────────────────────────────────────

pub struct CanLogPlayerKit;
pub static CAN_LOG_PLAYER_KIT: CanLogPlayerKit = CanLogPlayerKit;

static CAN_LOG_PLAYER_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "can-player",
    label: "CAN log player",
    summary: "Replays candump-format traffic into a bxCAN/FDCAN controller.",
    detail: "Host-side log player for J1939 / bus-monitor labs. Requires inline \
             `data:` (candump text) and optional ticks_per_second.",
    transport: Transport::Can,
    category: Category::Misc,
    config_keys: &[
        ConfigKey {
            name: "data",
            ty: ConfigType::Str,
            doc: "Inline candump .log text (required).",
        },
        ConfigKey {
            name: "ticks_per_second",
            ty: ConfigType::Int,
            doc: "Sim ticks per log second (default 1_000_000).",
        },
    ],
    labs: &[],
};

impl PeripheralKit for CanLogPlayerKit {
    fn metadata(&self) -> &'static KitMetadata {
        &CAN_LOG_PLAYER_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        require_can_connection(ctx, "can-player")?;
        let Some(data) = ctx.config_str("data") else {
            return Err(anyhow!(
                "can-player '{}': set 'path' (a candump .log file) or inline 'data'",
                ctx.device_id()
            ));
        };
        let tps = SystemBus::yaml_u32(ctx.ext.config.get("ticks_per_second"), 1_000_000) as u64;
        let player = CanLogPlayer::from_candump(
            ctx.device_id().to_string(),
            ctx.connection().to_string(),
            data,
            tps,
        )
        .map_err(|e| anyhow!(e))?;
        ctx.bus.can_log_players.push(player);
        Ok(())
    }
}
