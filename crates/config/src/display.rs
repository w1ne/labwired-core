// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#![allow(dead_code)]
use crate::*;

/// Assert what a display device actually PAINTED, over a bounded region of its
/// own pixel grid.
///
/// One primitive for every panel — ILI9341, SSD1306, SH1107, tri-color e-paper,
/// the parallel ILI9341 — because it is keyed by the `external_devices:` id and
/// reads the framebuffer artifact the model already publishes, never a
/// per-model accessor.
///
/// **Why a region plus an ink RANGE, and not a lit-pixel count.** Two real
/// failures had to be distinguishable, and a count only separates one of them:
///
/// * A panel that paints NOTHING (a declared-but-undriven D/C line latches low,
///   every byte frames as a command, not one pixel lands) has zero ink.
/// * A panel that paints the wrong thing — a desynchronised command stream
///   writing command bytes into frame memory as pixels — has plenty of ink, in
///   roughly the right place, and sails past any "did it paint?" threshold.
///
/// Bounding the region and bounding the ink from BOTH sides is what tells those
/// apart: a header band the firmware fills solid must come back essentially
/// fully inked, and noise in that band does not. `max_ink` is the half that
/// makes the second case fail, so it is not optional decoration — a region with
/// `min_ink: 0.0` and no `max_ink` asserts nothing at all and
/// [`TestScript::validate`] rejects it.
///
/// A digest of the whole framebuffer would also catch both, exactly, and was
/// rejected: it fails on any legitimate change, says nothing about WHERE the
/// picture went wrong, and cannot be written by hand from a datasheet or a
/// photograph of the real panel.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct DisplayRegionDetails {
    /// `external_devices:` id of the display (e.g. `"tft"`).
    pub id: String,
    /// Region origin, in the panel's own pixel coordinates. Defaults to (0, 0).
    #[serde(default)]
    pub x: usize,
    #[serde(default)]
    pub y: usize,
    /// Region size. Defaults to the rest of the panel from (`x`, `y`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<usize>,
    /// Lower bound on the fraction (0.0..=1.0) of the region's pixels that must
    /// carry ink — non-black on an emissive panel, non-white on e-paper.
    pub min_ink: f64,
    /// Upper bound on the same fraction. Absent means 1.0 (no upper bound),
    /// which is only allowed when `min_ink` is itself above zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ink: Option<f64>,
    /// Require the panel to be EMITTING, not merely painted.
    ///
    /// Ink measures frame memory, and frame memory fills whether or not the
    /// panel can show it. On an emissive display those are different
    /// questions: an AMOLED has no backlight and its brightness lives in the
    /// controller (DCS `WRDISBV`, reset 0x00), so firmware ported from a
    /// backlit TFT driver paints a perfect frame and displays black.
    ///
    /// This is not hypothetical and it is why the field exists: deleting the
    /// one `WRDISBV` write from the nRF54LM20A snake firmware left its lab
    /// passing 7/7, because every assertion measured pixels that had genuinely
    /// been written to a panel nobody could see.
    ///
    /// Only meaningful for a panel that publishes `meta.lit`; asking it of one
    /// that does not is an error rather than a pass, on the same principle as
    /// every other way of not-measuring here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lit: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct DisplayRegionAssertion {
    pub display_region: DisplayRegionDetails,
}

/// Structural guard for a `display_region` assertion.
///
/// The interesting clause is the last one. `min_ink: 0.0` with no `max_ink`
/// accepts every possible framebuffer, including one that was never written —
/// a gate that cannot fail, which is worse than no gate because it reads as
/// coverage. Both other clauses are ordinary range checks.
pub(crate) fn validate_display_region(index: usize, d: &DisplayRegionDetails) -> Result<()> {
    if d.id.trim().is_empty() {
        anyhow::bail!("assertions[{index}]: display_region.id cannot be empty");
    }
    for (name, v) in [("min_ink", Some(d.min_ink)), ("max_ink", d.max_ink)] {
        if let Some(v) = v {
            if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                anyhow::bail!(
                    "assertions[{index}]: display_region.{name} must be a fraction in 0.0..=1.0 (got {v})"
                );
            }
        }
    }
    if let Some(max) = d.max_ink {
        if max < d.min_ink {
            anyhow::bail!(
                "assertions[{index}]: display_region.max_ink ({max}) is below min_ink ({})",
                d.min_ink
            );
        }
    }
    if d.min_ink == 0.0 && d.max_ink.is_none() {
        anyhow::bail!(
            "assertions[{index}]: display_region with min_ink 0.0 and no max_ink accepts every \
             possible framebuffer, including one the firmware never wrote. Give it a floor \
             (min_ink) to prove the region was painted, or a ceiling (max_ink) to prove it was \
             left clear."
        );
    }
    Ok(())
}
