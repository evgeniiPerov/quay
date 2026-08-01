//! `quay diff <skill>` — read-only comparison of a locally installed skill
//! against the copy on a hub. Never writes.
//!
//! `quay add` reconciles at collision time and only looks at `SKILL.md`; this
//! compares the whole skill directory, so a change in `references/` or
//! `scripts/` is visible too.

use quay_core::{
    push_log::PushLog,
    reconcile::{
        diff::Diff,
        folder::{folder_report, Change, FileChange, FolderReport},
        harbor_history::GitHarborHistory,
        verdict::{SemverRel, Verdict},
    },
    scanner::scan_local,
    CloneFetcher, Config, RegistryFetcher,
};
use serde::Serialize;
use std::path::Path;

pub fn run(
    project: &Path,
    skill: &str,
    remote: Option<&str>,
    profile: Option<&str>,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;

    // Locate the install. Reporting "no differences" for a skill that is not
    // installed would be a lie dressed as a clean result.
    let log = PushLog::load(
        crate::config_io::default_config_dir()
            .as_deref()
            .unwrap_or(project),
        Some(project),
    )
    .unwrap_or_else(|e| {
        // Matches `commands::lock`: the push log only decorates scan status, so
        // a corrupt one must not fail the command — but it must not be silent
        // either.
        eprintln!("warning: could not read push log ({e}); treating as empty");
        PushLog::default()
    });
    let locals = scan_local(project, &log);
    let local = locals
        .iter()
        .find(|s| s.meta.name == skill)
        .ok_or_else(|| format!("skill '{skill}' is not installed in this project"))?;

    let (remote_name, remote_cfg) = pick_remote(&cfg, remote)?;

    let fetcher = CloneFetcher::new();
    let registry = match remote_cfg.direct_branch.as_deref() {
        Some(b) => fetcher.fetch_at(&remote_cfg.url, b)?,
        None => fetcher.fetch(&remote_cfg.url)?,
    };
    let entry = registry
        .entry(skill)
        .ok_or_else(|| format!("remote '{remote_name}' does not publish '{skill}'"))?;

    let harbor =
        GitHarborHistory::clone_harbor(&remote_cfg.url, remote_cfg.direct_branch.as_deref())?;
    let local_dir = local
        .canonical_path()
        .parent()
        .ok_or("SKILL.md has no parent directory")?;
    // Only the canonical copy is compared. Staying silent would let a report of
    // "up to date" hide edits sitting in a mirror this command never opened.
    if local.has_drift() {
        eprintln!(
            "warning: '{skill}' differs between mirror roots; comparing the canonical copy at {} only",
            local_dir.display()
        );
    }
    let report = folder_report(
        local_dir,
        &harbor,
        &entry.path,
        &entry.version,
        &local.meta.version,
    )?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&as_json(skill, &remote_name, &report))?
        );
    } else {
        print_human(skill, &remote_name, &report);
    }
    Ok(())
}

/// The named remote, or the configured default.
///
/// Resolution goes through `Config::default_remote` rather than picking a lone
/// remote that is not flagged `default`. Every other command (`add`, `push`,
/// `remove`, `rebuild-registry`) errors in that state, and diverging here would
/// let `quay diff` succeed and then recommend a `quay add --force` that fails.
fn pick_remote<'a>(
    cfg: &'a Config,
    requested: Option<&str>,
) -> Result<(String, &'a quay_core::RemoteConfig), Box<dyn std::error::Error>> {
    if let Some(name) = requested {
        let c = cfg
            .remotes
            .get(name)
            .ok_or_else(|| format!("no remote named '{name}'"))?;
        return Ok((name.to_string(), c));
    }
    if cfg.remotes.is_empty() {
        return Err("no remotes configured; add one with `quay remote add`".into());
    }
    let (n, c) = cfg
        .default_remote()
        .ok_or("no default remote configured — pass --remote=<name>")?;
    Ok((n.clone(), c))
}

fn print_human(skill: &str, remote: &str, report: &FolderReport) {
    println!("{skill}  (vs {remote})  {}", headline(report));
    if let Some(hint) = semver_note(report.semver) {
        println!("  {hint}");
    }

    if report.verdict == Verdict::Identical {
        return;
    }

    for f in &report.files {
        // Unchanged files carry no diff and are not listed.
        let Some(diff) = f.change.diff() else {
            continue;
        };
        println!("\n{} {}", mark(&f.change), f.rel);
        match diff {
            Diff::Text(t) => print!("{}", indent(t)),
            // Pull-oriented call site: `folder_report` renders
            // render(local, hub), so old = local and new = hub.
            Diff::Binary {
                old_bytes: local_bytes,
                new_bytes: hub_bytes,
            } => println!("    (binary: {local_bytes} bytes local, {hub_bytes} on hub)"),
        }
    }

    match report.verdict {
        // Only recommend overwriting when the hub is the side that moved.
        // Offering it for a local edit or an unresolved direction turns a
        // read-only report's single piece of advice into data loss.
        Verdict::HubNewer { .. } => {
            println!("\nTake the hub copy with: quay add {skill} --force");
        }
        // `AbsentOnHub` is deliberately not here: there is no hub copy to take.
        Verdict::LocalAheadOrDiverged { .. } | Verdict::ChangedUnknownDirection => {
            println!(
                "\nYour copy may hold local changes. `quay add {skill} --force` takes the hub's version; files outside the hub's manifest are kept."
            );
        }
        _ => {}
    }
}

/// Mirrors `folder::WALK_CAP` for the message below. Kept as a display-only
/// constant because the core cap is private; the report's
/// `base_search_truncated` flag is what actually drives the branch.
const WALK_CAP_DISPLAY: usize = 50;

fn headline(report: &FolderReport) -> String {
    match &report.verdict {
        Verdict::Identical => "up to date".into(),
        Verdict::HubNewer {
            commits_ahead,
            last_commit_date,
            ..
        } => format!("hub is ahead by {commits_ahead} commit(s), last {last_commit_date}"),
        Verdict::LocalAheadOrDiverged { .. } => "your copy is ahead of the hub".into(),
        Verdict::AbsentOnHub => "no longer on the hub (deleted or renamed there)".into(),
        Verdict::ChangedUnknownDirection if report.base_search_truncated => format!(
            "differs from the hub; no match in the last {WALK_CAP_DISPLAY} commits touching it, so the search was cut short rather than exhausted"
        ),
        Verdict::ChangedUnknownDirection => {
            "differs from the hub; no commit matches your copy, so the direction is unknown".into()
        }
    }
}

/// Frontmatter versions are advisory. Surfacing the relation is useful, but it
/// must never read as the verdict — a hub can publish new content at an
/// unchanged version, and a local copy can carry a higher version than the hub.
fn semver_note(rel: SemverRel) -> Option<&'static str> {
    match rel {
        SemverRel::HubHigher => Some("version: hub is higher"),
        SemverRel::LocalHigher => Some("version: yours is higher (content may still be older)"),
        // Distinct from Equal: a skill with no frontmatter `version` reaches
        // here, and silence would read as "the versions agree".
        SemverRel::Unparseable => Some("version: not comparable on one or both sides"),
        SemverRel::Equal => None,
    }
}

fn mark(change: &Change) -> &'static str {
    match change {
        Change::Same => " ",
        Change::Modified(_) => "M",
        Change::OnlyOnHub(_) => "+",
        Change::OnlyLocal(_) => "-",
    }
}

fn indent(body: &str) -> String {
    body.lines().map(|l| format!("    {l}\n")).collect()
}

#[derive(Serialize)]
struct DiffReport<'a> {
    skill: &'a str,
    remote: &'a str,
    verdict: &'static str,
    /// Absent on the hub's HEAD — deleted or renamed there.
    absent_on_hub: bool,
    /// The base-commit search hit its cap, so `changed_unknown_direction` here
    /// means "did not look far enough", not "your copy matches nothing".
    base_search_truncated: bool,
    /// LF-normalized digests, equal iff the copies match. The `_lf` suffix is
    /// load-bearing: these are NOT the `content_hash` a registry publishes, and
    /// comparing them to one agrees by luck on all-LF content and diverges on a
    /// Windows checkout.
    local_hash_lf: &'a str,
    hub_hash_lf: &'a str,
    files: Vec<FileJson<'a>>,
}

#[derive(Serialize)]
struct FileJson<'a> {
    path: &'a str,
    change: &'static str,
    /// Omitted for unchanged files, and for binary content — see `binary`,
    /// which distinguishes "no body because nothing changed" from "no body
    /// because the bytes are not text".
    #[serde(skip_serializing_if = "Option::is_none")]
    diff: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    binary: Option<BinaryJson>,
}

/// Sizes labelled by side, translated from `Diff::Binary`'s positional fields.
#[derive(Serialize)]
struct BinaryJson {
    local_bytes: usize,
    hub_bytes: usize,
}

fn as_json<'a>(skill: &'a str, remote: &'a str, report: &'a FolderReport) -> DiffReport<'a> {
    DiffReport {
        skill,
        remote,
        verdict: verdict_tag(&report.verdict),
        absent_on_hub: report.absent_on_hub(),
        base_search_truncated: report.base_search_truncated,
        local_hash_lf: &report.local_hash,
        hub_hash_lf: &report.head_hash,
        files: report.files.iter().map(file_json).collect(),
    }
}

fn file_json(f: &FileChange) -> FileJson<'_> {
    FileJson {
        path: &f.rel,
        change: match f.change {
            Change::Same => "same",
            Change::Modified(_) => "modified",
            Change::OnlyOnHub(_) => "only_on_hub",
            Change::OnlyLocal(_) => "only_local",
        },
        diff: match f.change.diff() {
            Some(Diff::Text(t)) => Some(t.as_str()),
            // Binary bodies go in `binary`, unchanged files have no body.
            Some(Diff::Binary { .. }) | None => None,
        },
        // folder_report renders render(local, hub): old = local, new = hub.
        binary: match f.change.diff() {
            Some(Diff::Binary {
                old_bytes,
                new_bytes,
            }) => Some(BinaryJson {
                local_bytes: *old_bytes,
                hub_bytes: *new_bytes,
            }),
            Some(Diff::Text(_)) | None => None,
        },
    }
}

fn verdict_tag(v: &Verdict) -> &'static str {
    match v {
        Verdict::Identical => "identical",
        Verdict::HubNewer { .. } => "hub_newer",
        Verdict::LocalAheadOrDiverged { .. } => "local_ahead_or_diverged",
        // `AbsentOnHub` shares this tag on purpose: it used to be a
        // `ChangedUnknownDirection` carrying a flag, and the `absent_on_hub`
        // field beside it already tells the two apart. Giving it a tag of its
        // own would break every consumer matching on `verdict`.
        Verdict::ChangedUnknownDirection | Verdict::AbsentOnHub => "changed_unknown_direction",
    }
}
