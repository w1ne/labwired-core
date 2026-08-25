// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Where a ratchet's baseline comes from.
//!
//! A ratchet compares "now" against "the last agreed state". The whole guarantee
//! rests on those being two DIFFERENT commits. Read the baseline out of the
//! working tree — `include_str!`, or just opening the file — and the gate
//! compares the tree to itself: a change that lowers the number and regenerates
//! the snapshot in the same commit passes, which is precisely the change the
//! ratchet exists to stop.
//!
//! `tier1` got this right and the SVD coverage ratchet did not, so the git dance
//! lives here once instead of twice: resolve a commit that is genuinely earlier
//! than HEAD, then read the file's blob AT that commit.
//!
//! Refusing is a legitimate answer. A shallow clone cannot resolve a merge base,
//! and a wrong baseline is indistinguishable from a green gate — so this returns
//! an error naming the fix rather than guessing.

use std::path::Path;

/// The ref baselines are taken against.
pub const DEFAULT_BASELINE_REF: &str = "origin/main";

/// Override for [`DEFAULT_BASELINE_REF`] — forks, release branches, and the
/// tests that must not mutate the process environment to pick one.
pub const BASELINE_REF_ENV: &str = "LABWIRED_BASELINE_REF";

/// The ref the baseline is taken against, honouring [`BASELINE_REF_ENV`].
pub fn baseline_ref() -> String {
    std::env::var(BASELINE_REF_ENV).unwrap_or_else(|_| DEFAULT_BASELINE_REF.to_string())
}

pub(crate) fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// What a baseline lookup found.
#[derive(Debug)]
pub struct Baseline {
    /// Short commit + how it was chosen, for the failure message.
    pub label: String,
    /// The file's contents at that commit. `None` means there is no earlier
    /// recorded state — the file is new — so there is nothing yet to protect.
    pub blob: Option<String>,
}

/// Resolve `path`'s contents at the baseline commit.
///
/// `what` names the gate in error messages, so a failure says which ratchet
/// could not establish its baseline.
pub fn resolve(root: &Path, base_ref: &str, path: &str, what: &str) -> Result<Baseline, String> {
    // A shallow clone cannot be trusted to resolve a merge base or to walk the
    // file's history, and a wrong baseline is indistinguishable from a green
    // gate. Fail loudly with the fix instead of guessing.
    if git(root, &["rev-parse", "--is-shallow-repository"])? == "true" {
        return Err(format!(
            "{what}: this is a SHALLOW clone, so the baseline cannot be established. \
             Deepen it first (`git fetch --unshallow origin` or `actions/checkout` with \
             `fetch-depth: 0`) and make sure `{base_ref}` exists. \
             Refusing to run: a gate that cannot find its baseline must not pass."
        ));
    }

    let head = git(root, &["rev-parse", "HEAD"])?;
    let merge_base = git(root, &["merge-base", "HEAD", base_ref]).map_err(|e| {
        format!(
            "{what}: cannot compute merge-base(HEAD, {base_ref}): {e}. \
             Fetch the baseline ref (`git fetch --no-tags origin \
             +refs/heads/main:refs/remotes/origin/main`) or point {BASELINE_REF_ENV} \
             at a ref that exists. Refusing to run without a baseline."
        )
    })?;

    let (commit, how) = if merge_base != head {
        (merge_base.clone(), format!("merge-base with {base_ref}"))
    } else {
        // On the trunk: the newest commit that touched the file, and the one
        // before it.
        let log = git(
            root,
            &["log", "--format=%H", "--first-parent", "HEAD", "--", path],
        )?;
        let revs: Vec<&str> = log.lines().collect();
        match revs.first() {
            // HEAD itself changed the file — measure against what it replaced.
            Some(&newest) if newest == head => match revs.get(1) {
                Some(&prev) => (
                    prev.to_string(),
                    "previous recorded state (HEAD is on the baseline branch)".to_string(),
                ),
                None => {
                    return Ok(Baseline {
                        label: "no prior revision (file introduced by HEAD)".to_string(),
                        blob: None,
                    })
                }
            },
            // Unchanged at HEAD: nothing new to ratchet.
            Some(&newest) => (
                newest.to_string(),
                "newest recorded state (unchanged at HEAD)".to_string(),
            ),
            None => {
                return Ok(Baseline {
                    label: "no recorded revision".to_string(),
                    blob: None,
                })
            }
        }
    };

    // The path may not exist at the baseline at all — a file introduced on this
    // branch. That is "nothing promised yet", NOT a broken gate, so it must be
    // distinguished from a git failure rather than reported as one. Ask first:
    // `git show` on a missing path and `git show` on a broken repo both exit
    // non-zero, and collapsing the two would turn a new file into a hard error.
    if git(root, &["cat-file", "-e", &format!("{commit}:{path}")]).is_err() {
        return Ok(Baseline {
            label: format!(
                "{} (path absent at baseline)",
                &commit[..commit.len().min(12)]
            ),
            blob: None,
        });
    }
    let blob = git(root, &["show", &format!("{commit}:{path}")])
        .map_err(|e| format!("{what}: cannot read {path} at baseline {commit}: {e}"))?;
    Ok(Baseline {
        label: format!("{} ({how})", &commit[..commit.len().min(12)]),
        blob: Some(blob),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("baseline-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
        ] {
            git(&dir, &args).unwrap();
        }
        dir
    }

    fn commit(dir: &std::path::Path, path: &str, body: &str, msg: &str) -> String {
        std::fs::write(dir.join(path), body).unwrap();
        git(dir, &["add", "-A"]).unwrap();
        git(dir, &["commit", "-qm", msg, "--no-verify"]).unwrap();
        git(dir, &["rev-parse", "HEAD"]).unwrap()
    }

    /// THE DEFECT THIS MODULE EXISTS TO PREVENT. A ratchet that reads its
    /// baseline from the working tree grades a commit against itself, so a
    /// change that lowers the number AND rewrites the snapshot in one commit
    /// passes. The baseline must be the blob at an EARLIER commit.
    #[test]
    fn reads_the_earlier_commit_not_the_working_tree() {
        let dir = seed("earlier");
        let base = commit(&dir, "snap.json", "{\"v\":1}", "baseline");
        git(&dir, &["branch", "trunk"]).unwrap();
        git(&dir, &["checkout", "-q", "-b", "work"]).unwrap();
        // The shape that used to slip through: change the artifact on the branch.
        commit(
            &dir,
            "snap.json",
            "{\"v\":0}",
            "lower it and rewrite the snapshot",
        );

        let found = resolve(&dir, "trunk", "snap.json", "test gate").unwrap();
        assert_eq!(
            found.blob.as_deref(),
            Some("{\"v\":1}"),
            "baseline must be the blob at {base}, not what the working tree says"
        );
        assert!(found.label.contains("merge-base"), "{}", found.label);
    }

    /// A file introduced by HEAD has no earlier state, so there is nothing to
    /// protect — and that must be said, not reported as an empty baseline the
    /// caller mistakes for "no regressions".
    #[test]
    fn says_so_when_the_file_is_new() {
        let dir = seed("new");
        commit(&dir, "other", "x", "seed");
        git(&dir, &["branch", "trunk"]).unwrap();
        commit(&dir, "snap.json", "{\"v\":1}", "introduce the snapshot");

        let found = resolve(&dir, "trunk", "snap.json", "test gate").unwrap();
        assert!(found.blob.is_none(), "expected no baseline blob");
    }

    /// A shallow clone cannot resolve a merge base, and a wrong baseline is
    /// indistinguishable from a green gate.
    #[test]
    fn refuses_a_shallow_clone() {
        let dir = seed("shallow");
        commit(&dir, "snap.json", "{}", "seed");
        std::fs::write(dir.join(".git/shallow"), "").unwrap();

        let err = resolve(&dir, DEFAULT_BASELINE_REF, "snap.json", "test gate")
            .expect_err("a shallow clone must be an error, never a silent pass");
        assert!(err.contains("SHALLOW"), "{err}");
    }
}
