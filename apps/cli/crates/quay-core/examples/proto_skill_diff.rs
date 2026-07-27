//! PROTOTYPE — throwaway. Answers: "what should `quay diff <skill>` report,
//! and does a folder-level verdict feel right?"
//!
//! Run: cargo run -p quay-core --example proto_skill_diff
//!
//! Not production. No I/O against a real harbor, no persistence. Every
//! scenario is an in-memory fake harbor + fake local tree, pushed through a
//! PROPOSED folder-level reconcile built on the existing pure pieces
//! (`reconcile::diff::render`, `reconcile::verdict::{classify, semver_hint}`,
//! `reconcile::baseline::{derive, content_sha256}`).
//!
//! What it deliberately does NOT reuse: `reconcile::reconcile()` itself, which
//! is hard-wired to a single `SKILL.md`. Whether that limit is acceptable is
//! the question this prototype exists to settle.

// Throwaway fixtures use nested tuples; not worth naming types for code that
// gets deleted once the decision is made.
#![allow(clippy::type_complexity)]

use quay_core::error::Result;
use quay_core::reconcile::diff::{render, Diff};
use quay_core::reconcile::harbor_history::{Commit, HarborHistory};
use quay_core::reconcile::verdict::{BaseFacts, BasePosition};
// FINDING: `harbor_history` re-imports `CommitId` privately, so an external
// impl of `HarborHistory` must reach into `verdict` for it.
use quay_core::reconcile::verdict::{classify, semver_hint, CommitId, SemverRel, Verdict};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;

// ---------------------------------------------------------------- fake harbor

/// Newest-first commit chain; `blobs[(commit, path)]` is the file at that rev.
struct FakeHarbor {
    chain: Vec<Commit>,
    blobs: HashMap<(String, String), Vec<u8>>,
}

impl FakeHarbor {
    fn resolve(&self, rev: &str) -> String {
        if rev == "HEAD" {
            self.chain.first().map(|c| c.id.clone()).unwrap_or_default()
        } else {
            rev.to_string()
        }
    }

    /// FINDING: `HarborHistory` has no listing method. A folder diff needs one
    /// (real impl: `git ls-tree -r --name-only <rev> -- <prefix>`).
    fn paths_at(&self, rev: &str, prefix: &str) -> Vec<String> {
        let id = self.resolve(rev);
        let mut out: Vec<String> = self
            .blobs
            .keys()
            .filter(|(c, p)| *c == id && p.starts_with(prefix))
            .map(|(_, p)| p.clone())
            .collect();
        out.sort();
        out
    }

    /// Whole skill tree at `rev`, keyed relative to the skill dir.
    fn tree_at(&self, rev: &str, prefix: &str) -> Result<BTreeMap<String, Vec<u8>>> {
        let mut out = BTreeMap::new();
        for p in self.paths_at(rev, prefix) {
            let rel = p
                .trim_start_matches(prefix)
                .trim_start_matches('/')
                .to_string();
            if let Some(b) = self.bytes_at(rev, &p)? {
                out.insert(rel, b);
            }
        }
        Ok(out)
    }

    /// Commits touching anything under the skill dir (real impl:
    /// `git log -- <prefix>`), newest-first.
    fn commits_touching_dir(&self, prefix: &str) -> Vec<Commit> {
        self.chain
            .iter()
            .filter(|c| {
                self.blobs
                    .keys()
                    .any(|(cid, p)| cid == &c.id && p.starts_with(prefix))
            })
            .cloned()
            .collect()
    }
}

impl HarborHistory for FakeHarbor {
    fn head_sha(&self) -> Result<CommitId> {
        Ok(self.resolve("HEAD"))
    }
    fn bytes_at(&self, rev: &str, skill_path: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .blobs
            .get(&(self.resolve(rev), skill_path.to_string()))
            .cloned())
    }
    fn commits_touching(&self, skill_path: &str) -> Result<Vec<Commit>> {
        Ok(self
            .chain
            .iter()
            .filter(|c| {
                self.blobs
                    .contains_key(&(c.id.clone(), skill_path.to_string()))
            })
            .cloned()
            .collect())
    }
    fn is_ancestor(&self, a: &CommitId, b: &CommitId) -> Result<bool> {
        // Newest-first chain: a is an ancestor of b iff it sits later in the vec.
        let pos = |id: &str| self.chain.iter().position(|c| c.id == id);
        Ok(match (pos(a), pos(b)) {
            (Some(ia), Some(ib)) => ia > ib,
            _ => false,
        })
    }
}

// ------------------------------------------------------- proposed folder model

#[derive(Debug, PartialEq, Eq)]
enum FileKind {
    Same,
    Modified,
    AddedOnHub,
    RemovedOnHub,
}

struct FileChange {
    rel: String,
    kind: FileKind,
    diff: Option<Diff>,
}

struct FolderReport {
    /// Rollup verdict, computed on the FOLDER hash rather than SKILL.md alone.
    verdict: Verdict,
    semver: SemverRel,
    /// True when SKILL.md is byte-identical but sibling files differ — the case
    /// today's SKILL.md-only reconcile reports as `Identical`.
    hidden_from_skill_md_only: bool,
    files: Vec<FileChange>,
    local_folder_sha: String,
    head_folder_sha: String,
}

/// Same shape as `lock_hash::folder_hash` but over in-memory maps.
fn folder_sha(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut h = Sha256::new();
    for (rel, content) in files {
        h.update(rel.as_bytes());
        h.update(content);
    }
    hex::encode(h.finalize())
}

/// Folder-hash analogue of `baseline::derive`. Walks commits touching the skill
/// DIRECTORY (not just SKILL.md), hashing the whole tree at each rev until one
/// matches the local folder hash.
fn derive_folder(
    local_folder_sha: &str,
    harbor: &FakeHarbor,
    prefix: &str,
    head_folder_sha: &str,
) -> Result<Option<BaseFacts>> {
    if local_folder_sha == head_folder_sha {
        return Ok(None); // identical: classify short-circuits before using base
    }
    let head = harbor.head_sha()?;
    let commits = harbor.commits_touching_dir(prefix);
    for commit in commits.iter().take(WALK_CAP) {
        let tree = harbor.tree_at(&commit.id, prefix)?;
        if folder_sha(&tree) != local_folder_sha {
            continue;
        }
        let position = if harbor.is_ancestor(&commit.id, &head)? {
            BasePosition::AncestorOfHead {
                commits_ahead: commits.iter().take_while(|c| c.id != commit.id).count() as u32,
                last_commit_date: commits
                    .first()
                    .map(|c| c.committed_date.clone())
                    .unwrap_or_default(),
            }
        } else {
            BasePosition::HeadAncestorOfBase
        };
        return Ok(Some(BaseFacts {
            base: commit.id.clone(),
            position,
        }));
    }
    Ok(None)
}

fn folder_report(
    harbor: &FakeHarbor,
    prefix: &str,
    local: &BTreeMap<String, Vec<u8>>,
    hub_version: &str,
    local_version: &str,
) -> Result<FolderReport> {
    // Hub HEAD tree for this skill, keyed by path relative to the skill dir.
    let mut head: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for p in harbor.paths_at("HEAD", prefix) {
        let rel = p
            .trim_start_matches(prefix)
            .trim_start_matches('/')
            .to_string();
        if let Some(b) = harbor.bytes_at("HEAD", &p)? {
            head.insert(rel, b);
        }
    }

    let mut files = Vec::new();
    let mut rels: Vec<&String> = head.keys().chain(local.keys()).collect();
    rels.sort();
    rels.dedup();
    for rel in rels {
        let (h, l) = (head.get(rel), local.get(rel));
        let (kind, diff) = match (h, l) {
            (Some(h), Some(l)) if h == l => (FileKind::Same, None),
            (Some(h), Some(l)) => (FileKind::Modified, Some(render(h, l))),
            (Some(h), None) => (FileKind::AddedOnHub, Some(render(h, b""))),
            (None, Some(l)) => (FileKind::RemovedOnHub, Some(render(b"", l))),
            (None, None) => unreachable!(),
        };
        files.push(FileChange {
            rel: rel.clone(),
            kind,
            diff,
        });
    }

    let local_folder_sha = folder_sha(local);
    let head_folder_sha = folder_sha(&head);

    // FINDING (the whole point of this prototype): base derivation must run in
    // the SAME hash space as `classify`. Feeding folder hashes to a base
    // derived from SKILL.md history yields a false
    // `ChangedUnknownDirection { local_edited: true }` whenever SKILL.md is
    // untouched but a sibling file moved. So derive over folder hashes.
    let base = derive_folder(&local_folder_sha, harbor, prefix, &head_folder_sha)?;
    // FINDING: an empty tree hashes to the sha256 of nothing, which is a real
    // value — so "skill absent on HEAD" must be detected by listing, not by
    // hash. `mod.rs` gets this right for one file via `head_bytes: None`; the
    // folder version needs the same explicit check.
    let absent_on_head = head.is_empty();
    let verdict = if absent_on_head {
        Verdict::ChangedUnknownDirection {
            local_edited: false,
        }
    } else {
        classify(&local_folder_sha, &head_folder_sha, base)
    };

    let skill_md_same = matches!(
        files.iter().find(|f| f.rel == "SKILL.md"),
        Some(FileChange {
            kind: FileKind::Same,
            ..
        })
    );
    let others_differ = files
        .iter()
        .any(|f| f.rel != "SKILL.md" && f.kind != FileKind::Same);

    Ok(FolderReport {
        verdict,
        semver: semver_hint(hub_version, local_version),
        hidden_from_skill_md_only: skill_md_same && others_differ,
        files,
        local_folder_sha,
        head_folder_sha,
    })
}

// ------------------------------------------------------------------- rendering

fn headline(r: &FolderReport) -> String {
    match &r.verdict {
        Verdict::Identical => "up to date".into(),
        Verdict::HubNewer {
            commits_ahead,
            last_commit_date,
            ..
        } => format!("harbor is ahead by {commits_ahead} commit(s), last {last_commit_date}"),
        Verdict::LocalAheadOrDiverged { .. } => {
            "your copy is ahead of / diverged from harbor".into()
        }
        Verdict::ChangedUnknownDirection { local_edited } => {
            format!("changed, direction unknown (local_edited={local_edited})")
        }
    }
}

fn print_report(name: &str, r: &FolderReport, show_diffs: bool) {
    println!("\n=== {name}: {} ===", headline(r));
    println!("  semver hint      : {:?}", r.semver);
    println!("  local folder sha : {}", &r.local_folder_sha[..12]);
    println!("  harbor folder sha: {}", &r.head_folder_sha[..12]);
    if r.hidden_from_skill_md_only {
        println!("  !! SKILL.md is identical — today's reconcile would say 'Identical'");
    }
    let changed = r.files.iter().filter(|f| f.kind != FileKind::Same).count();
    println!(
        "  files            : {} total, {changed} changed",
        r.files.len()
    );
    for f in &r.files {
        let mark = match f.kind {
            FileKind::Same => " ",
            FileKind::Modified => "M",
            FileKind::AddedOnHub => "+",
            FileKind::RemovedOnHub => "-",
        };
        let extra = match &f.diff {
            Some(Diff::Binary {
                hub_bytes,
                local_bytes,
            }) => format!("  (binary {hub_bytes}B hub / {local_bytes}B local)"),
            _ => String::new(),
        };
        println!("   {mark} {}{extra}", f.rel);
    }
    if show_diffs {
        for f in &r.files {
            if let Some(Diff::Text(t)) = &f.diff {
                if f.kind != FileKind::Same {
                    println!("\n--- {} ---\n{}", f.rel, t.trim_end());
                }
            }
        }
    }
}

// ------------------------------------------------------------------- scenarios

struct Scenario {
    name: &'static str,
    question: &'static str,
    /// Newest-first: (commit id, date, files as (rel path, content)).
    history: Vec<(
        &'static str,
        &'static str,
        Vec<(&'static str, &'static [u8])>,
    )>,
    local: Vec<(&'static str, &'static [u8])>,
    hub_version: &'static str,
    local_version: &'static str,
}

const PREFIX: &str = "skills/foo";
/// Mirrors `baseline::WALK_CAP` (private there).
const WALK_CAP: usize = 50;

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "hub edited, version NOT bumped",
            question: "today `outdated` compares semver for frontmatter skills — silent",
            history: vec![
                ("c2", "2026-07-20", vec![("SKILL.md", b"body v2\n")]),
                ("c1", "2026-07-01", vec![("SKILL.md", b"body v1\n")]),
            ],
            local: vec![("SKILL.md", b"body v1\n")],
            hub_version: "1.0.0",
            local_version: "1.0.0",
        },
        Scenario {
            name: "SKILL.md same, references/ changed on hub",
            question: "SKILL.md-only reconcile cannot see this at all",
            history: vec![
                (
                    "c2",
                    "2026-07-22",
                    vec![
                        ("SKILL.md", b"body\n"),
                        ("references/api.md", b"GET /v2/things\n"),
                    ],
                ),
                (
                    "c1",
                    "2026-07-01",
                    vec![
                        ("SKILL.md", b"body\n"),
                        ("references/api.md", b"GET /v1/things\n"),
                    ],
                ),
            ],
            local: vec![
                ("SKILL.md", b"body\n"),
                ("references/api.md", b"GET /v1/things\n"),
            ],
            hub_version: "1.0.0",
            local_version: "1.0.0",
        },
        Scenario {
            name: "local edited, hub untouched",
            question: "should a read-only diff nag, or stay quiet?",
            history: vec![("c1", "2026-07-01", vec![("SKILL.md", b"body\n")])],
            local: vec![("SKILL.md", b"body\nmy local tweak\n")],
            hub_version: "1.0.0",
            local_version: "1.0.0",
        },
        Scenario {
            name: "both edited (true divergence)",
            question: "no base commit matches — can we still say anything useful?",
            history: vec![
                ("c2", "2026-07-20", vec![("SKILL.md", b"hub rewrite\n")]),
                ("c1", "2026-07-01", vec![("SKILL.md", b"body\n")]),
            ],
            local: vec![("SKILL.md", b"body\nmy local tweak\n")],
            hub_version: "1.1.0",
            local_version: "1.0.0",
        },
        Scenario {
            name: "hub added a file, hub deleted a file",
            question: "add/remove need distinct marks, not a diff body",
            history: vec![(
                "c1",
                "2026-07-20",
                vec![("SKILL.md", b"body\n"), ("scripts/new.sh", b"echo hi\n")],
            )],
            local: vec![("SKILL.md", b"body\n"), ("old.md", b"stale\n")],
            hub_version: "1.1.0",
            local_version: "1.0.0",
        },
        Scenario {
            name: "binary asset changed",
            question: "diff body is useless; byte counts only",
            history: vec![(
                "c1",
                "2026-07-20",
                vec![
                    ("SKILL.md", b"body\n"),
                    ("img.png", &[0xff, 0xd8, 0xff, 0x01]),
                ],
            )],
            local: vec![("SKILL.md", b"body\n"), ("img.png", &[0xff, 0xd8, 0xff])],
            hub_version: "1.0.0",
            local_version: "1.0.0",
        },
        Scenario {
            name: "local version HIGHER but hub content is newer",
            question: "semver lies — does the verdict override the hint clearly?",
            history: vec![
                ("c2", "2026-07-20", vec![("SKILL.md", b"hub newer\n")]),
                ("c1", "2026-07-01", vec![("SKILL.md", b"body\n")]),
            ],
            local: vec![("SKILL.md", b"body\n")],
            hub_version: "1.0.0",
            local_version: "2.0.0",
        },
        Scenario {
            name: "skill gone from hub HEAD",
            question: "renamed or deleted upstream — what does pull say?",
            history: vec![("c1", "2026-07-20", vec![])],
            local: vec![("SKILL.md", b"body\n")],
            hub_version: "1.0.0",
            local_version: "1.0.0",
        },
    ]
}

fn build(s: &Scenario) -> (FakeHarbor, BTreeMap<String, Vec<u8>>) {
    let mut chain = Vec::new();
    let mut blobs = HashMap::new();
    for (id, date, files) in &s.history {
        chain.push(Commit {
            id: (*id).into(),
            committed_date: (*date).into(),
        });
        for (rel, content) in files {
            blobs.insert(
                ((*id).to_string(), format!("{PREFIX}/{rel}")),
                content.to_vec(),
            );
        }
    }
    let local = s
        .local
        .iter()
        .map(|(rel, c)| ((*rel).to_string(), c.to_vec()))
        .collect();
    (FakeHarbor { chain, blobs }, local)
}

fn run(s: &Scenario, show_diffs: bool) {
    let (harbor, local) = build(s);
    println!("\nQ: {}", s.question);
    match folder_report(&harbor, PREFIX, &local, s.hub_version, s.local_version) {
        Ok(r) => print_report(s.name, &r, show_diffs),
        Err(e) => println!("  ERROR: {e}"),
    }
}

fn main() {
    let list = scenarios();
    let mut show_diffs = false;
    println!("PROTOTYPE: quay diff — folder-level local vs harbor");
    loop {
        println!(
            "\n-- scenarios (diff bodies: {}) --",
            if show_diffs { "ON" } else { "off" }
        );
        for (i, s) in list.iter().enumerate() {
            println!("  {i}. {}", s.name);
        }
        println!("  a. all    d. toggle diff bodies    q. quit");
        print!("> ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        match line.trim() {
            "q" => return,
            "d" => show_diffs = !show_diffs,
            "a" => list.iter().for_each(|s| run(s, show_diffs)),
            n => match n.parse::<usize>().ok().and_then(|i| list.get(i)) {
                Some(s) => run(s, show_diffs),
                None => println!("?"),
            },
        }
    }
}
