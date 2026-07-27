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
        folder::{folder_report, ChangeKind, FileChange, FolderReport},
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
    .unwrap_or_default();
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

/// The named remote, or the default one. Ambiguity is an error rather than a
/// guess: diffing against the wrong hub silently is worse than asking.
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
    if let Some((n, c)) = cfg.remotes.iter().find(|(_, c)| c.default) {
        return Ok((n.clone(), c));
    }
    match cfg.remotes.len() {
        0 => Err("no remotes configured; add one with `quay remote add`".into()),
        1 => {
            let (n, c) = cfg.remotes.iter().next().expect("len == 1");
            Ok((n.clone(), c))
        }
        _ => Err("several remotes configured and none is default; pass --remote <name>".into()),
    }
}

fn print_human(skill: &str, remote: &str, report: &FolderReport) {
    println!("{skill}  (vs {remote})  {}", headline(report));
    if let Some(hint) = semver_note(report.semver) {
        println!("  {hint}");
    }

    if report.verdict == Verdict::Identical {
        return;
    }

    for f in report.files.iter().filter(|f| f.kind != ChangeKind::Same) {
        println!("\n{} {}", mark(f.kind), f.rel);
        match &f.diff {
            Some(Diff::Text(t)) => print!("{}", indent(t)),
            Some(Diff::Binary {
                hub_bytes,
                local_bytes,
            }) => println!("    (binary: {local_bytes} bytes local, {hub_bytes} on hub)"),
            None => {}
        }
    }

    if !report.absent_on_head {
        println!("\nTake the hub copy with: quay add {skill} --force");
    }
}

fn headline(report: &FolderReport) -> String {
    match &report.verdict {
        Verdict::Identical => "up to date".into(),
        Verdict::HubNewer {
            commits_ahead,
            last_commit_date,
            ..
        } => format!("hub is ahead by {commits_ahead} commit(s), last {last_commit_date}"),
        Verdict::LocalAheadOrDiverged { .. } => "your copy is ahead of the hub".into(),
        Verdict::ChangedUnknownDirection { local_edited } if report.absent_on_head => {
            let _ = local_edited;
            "no longer on the hub (deleted or renamed there)".into()
        }
        Verdict::ChangedUnknownDirection { .. } => {
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
        SemverRel::Equal | SemverRel::Unparseable => None,
    }
}

fn mark(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Same => " ",
        ChangeKind::Modified => "M",
        ChangeKind::OnlyOnHub => "+",
        ChangeKind::OnlyLocal => "-",
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
    local_hash: &'a str,
    hub_hash: &'a str,
    files: Vec<FileJson<'a>>,
}

#[derive(Serialize)]
struct FileJson<'a> {
    path: &'a str,
    change: &'static str,
    /// Omitted for unchanged files and for binary content.
    #[serde(skip_serializing_if = "Option::is_none")]
    diff: Option<&'a str>,
}

fn as_json<'a>(skill: &'a str, remote: &'a str, report: &'a FolderReport) -> DiffReport<'a> {
    DiffReport {
        skill,
        remote,
        verdict: verdict_tag(&report.verdict),
        absent_on_hub: report.absent_on_head,
        local_hash: &report.local_hash,
        hub_hash: &report.head_hash,
        files: report.files.iter().map(file_json).collect(),
    }
}

fn file_json(f: &FileChange) -> FileJson<'_> {
    FileJson {
        path: &f.rel,
        change: match f.kind {
            ChangeKind::Same => "same",
            ChangeKind::Modified => "modified",
            ChangeKind::OnlyOnHub => "only_on_hub",
            ChangeKind::OnlyLocal => "only_local",
        },
        diff: match &f.diff {
            Some(Diff::Text(t)) => Some(t.as_str()),
            _ => None,
        },
    }
}

fn verdict_tag(v: &Verdict) -> &'static str {
    match v {
        Verdict::Identical => "identical",
        Verdict::HubNewer { .. } => "hub_newer",
        Verdict::LocalAheadOrDiverged { .. } => "local_ahead_or_diverged",
        Verdict::ChangedUnknownDirection { .. } => "changed_unknown_direction",
    }
}
