//! Orchestrates the local-skill → hub-PR pipeline.
//!
//! [`SkillPusher`] is generic over [`GitClient`] and [`PrOpener`] so tests can
//! inject fakes without spawning real git processes or making network calls.

use crate::config::Config;
use crate::error::{QuayError, Result};
use crate::git::GitClient;
use crate::manifest::SkillManifest;
use crate::provider::{PrInfo, PrOpener};
use crate::push_log::{PushLog, PushRecord};
use crate::scanner::{parse_skill_metadata, SkillFormat};
use semver::Version;
use std::path::{Path, PathBuf};

/// What kind of bump to apply to the existing hub-side version on a push.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpKind {
    Patch,
    Minor,
    Major,
    /// Use the version that's already in the local SKILL.md frontmatter as-is.
    AsWritten,
}

/// Outcome of a successful push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushResult {
    pub remote: String,
    pub branch: String,
    pub version: String,
    /// `Some(...)` for PR-mode pushes, `None` for direct pushes.
    pub pr: Option<PrInfo>,
    /// SHA of the new commit on the hub.  Always populated regardless of mode.
    pub commit_sha: String,
}

/// Drives the local-skill → hub-PR pipeline.
pub struct SkillPusher<'a, G: GitClient, P: PrOpener> {
    pub config: &'a Config,
    pub git: &'a G,
    pub opener: &'a P,
    pub project_root: PathBuf,
    /// Directory that holds the user-level `config.toml` (e.g. `~/.config/quay/`).
    ///
    /// When `Some`, push records are written to `<config_dir>/push-log.json`.
    /// When `None`, the push-log write is silently skipped.
    pub config_dir: Option<PathBuf>,
    /// Author identity for the commit. Falls back to the user-section in config when `None`.
    pub author: Option<(String, String)>,
}

impl<'a, G: GitClient, P: PrOpener> SkillPusher<'a, G, P> {
    /// Run the full push pipeline for `skill_name`:
    ///
    /// 1. Resolve the target remote.
    /// 2. Read the local `SKILL.md`.
    /// 3. Apply the requested version bump.
    /// 4. Clone the hub.
    /// 5. Write the updated manifest + body into the clone.
    /// 6. Copy any extra local files alongside `SKILL.md`.
    /// 7. Branch / commit / push.
    /// 8. Open a PR and return the result.
    pub fn push(
        &self,
        skill_name: &str,
        target_remote: Option<&str>,
        bump: BumpKind,
        clone_dest_root: &Path,
        push_mode_override: Option<crate::config::PushMode>,
        direct_branch_override: Option<&str>,
    ) -> Result<PushResult> {
        // 1. Resolve target remote.
        let remote_name = match target_remote {
            Some(name) => {
                if !self.config.remotes.contains_key(name) {
                    return Err(QuayError::RemoteUnknown(name.into()));
                }
                name.to_string()
            }
            None => match self.config.default_remote() {
                Some((name, _)) => name.clone(),
                None => {
                    return Err(QuayError::ConfigValidation(
                        "no default remote — pass --remote=<name>".into(),
                    ));
                }
            },
        };
        let remote_cfg = &self.config.remotes[&remote_name];

        // 2. Read local skill.
        let local_skill_dir = self.project_root.join(".agents/skills").join(skill_name);
        if !local_skill_dir.exists() {
            return Err(QuayError::SkillNotFound {
                name: skill_name.into(),
                remote: "local".into(),
            });
        }
        let local_md_path = local_skill_dir.join("SKILL.md");
        let raw = std::fs::read_to_string(&local_md_path).map_err(|source| QuayError::Io {
            path: local_md_path.display().to_string(),
            source,
        })?;
        let meta = parse_skill_metadata(&raw, &local_md_path);

        // Build a SkillManifest from lenient meta. Push uses this to populate registry.json
        // and to format the PR body. The on-disk skill file is untouched (raw bytes through)
        // EXCEPT for Frontmatter format with a bump request, which rewrites the version line
        // in-place — see the bump branch below.
        let mut manifest = SkillManifest {
            name: meta.name.clone(),
            description: meta.description.clone(),
            version: meta.version.clone(),
            category: None,
            tags: meta.tags.clone(),
            author: None,
            license: None,
            quay: Default::default(),
            source_format: meta.format,
        };

        // 3. Apply version bump (in memory; written on commit).
        match bump {
            BumpKind::AsWritten => {}
            BumpKind::Patch | BumpKind::Minor | BumpKind::Major => {
                if !matches!(meta.format, SkillFormat::Frontmatter) {
                    return Err(QuayError::ConfigValidation(format!(
                        "cannot apply --bump to a {} skill — version bumps require YAML frontmatter; \
                         leave bump as-written or convert the skill to canonical format first",
                        match meta.format {
                            SkillFormat::SlashCommand => "slash-command",
                            SkillFormat::Freestyle => "freestyle",
                            SkillFormat::Frontmatter => unreachable!(),
                        }
                    )));
                }
                let mut v = Version::parse(&manifest.version).map_err(|e| {
                    QuayError::InvalidFrontmatter {
                        path: local_md_path.display().to_string(),
                        reason: format!("version is not valid semver: {}", e),
                    }
                })?;
                match bump {
                    BumpKind::Patch => v.patch += 1,
                    BumpKind::Minor => {
                        v.minor += 1;
                        v.patch = 0;
                    }
                    BumpKind::Major => {
                        v.major += 1;
                        v.minor = 0;
                        v.patch = 0;
                    }
                    BumpKind::AsWritten => unreachable!(),
                }
                manifest.version = v.to_string();
            }
        }

        // Compute push mode + direct-branch target now (before the clone) so we
        // can shallow-clone the right branch and avoid non-fast-forward pushes
        // when a remote `direct_branch` is ahead of the hub's default branch.
        use crate::config::PushMode;
        let effective_mode = push_mode_override.unwrap_or(remote_cfg.push_mode);
        let effective_direct_branch: Option<&str> =
            direct_branch_override.or(remote_cfg.direct_branch.as_deref());

        // 4. Clone the hub.
        let hub_clone = clone_dest_root.join(format!("hub-{}", skill_name));
        if hub_clone.exists() {
            std::fs::remove_dir_all(&hub_clone).map_err(|source| QuayError::Io {
                path: hub_clone.display().to_string(),
                source,
            })?;
        }
        // When pushing to a non-default branch in direct mode, clone that
        // branch directly so the working tree starts on it and tracks
        // `origin/<branch>`. If the branch does not exist on the remote yet,
        // fall back to a default-branch clone — `finish_push_direct` will
        // auto-create the branch from current HEAD via `checkout -B`.
        let clone_branch = match effective_mode {
            PushMode::Direct => effective_direct_branch,
            PushMode::Pr => None,
        };
        match self.git.clone(&remote_cfg.url, &hub_clone, clone_branch) {
            Ok(()) => {}
            Err(_) if clone_branch.is_some() => {
                if hub_clone.exists() {
                    std::fs::remove_dir_all(&hub_clone).map_err(|source| QuayError::Io {
                        path: hub_clone.display().to_string(),
                        source,
                    })?;
                }
                self.git.clone(&remote_cfg.url, &hub_clone, None)?;
            }
            Err(e) => return Err(e),
        }

        // 5. Make sure target dir exists in the hub clone (default to flat layout if new).
        let hub_skill_dir = hub_clone.join("skills").join(skill_name);
        // Check before create_dir_all: if SKILL.md already exists, this is an update.
        // The exists() call is safe on a non-existent path — returns false.
        let is_new_skill = !hub_skill_dir.join("SKILL.md").exists();
        std::fs::create_dir_all(&hub_skill_dir).map_err(|source| QuayError::Io {
            path: hub_skill_dir.display().to_string(),
            source,
        })?;

        // 6. Write the file. For Frontmatter + bump, we re-emit normalized YAML frontmatter
        // so the on-hub file's version matches the bumped version. For all other cases
        // (Frontmatter without bump, SlashCommand, Freestyle), we copy raw bytes through.
        let bytes_to_write: Vec<u8> = if matches!(meta.format, SkillFormat::Frontmatter)
            && !matches!(bump, BumpKind::AsWritten)
        {
            // Re-emit only the frontmatter; preserve the body verbatim.
            let body = strip_frontmatter(&raw).unwrap_or("");
            let yaml =
                serde_yaml::to_string(&manifest).map_err(|e| QuayError::InvalidFrontmatter {
                    path: hub_skill_dir.display().to_string(),
                    reason: format!("could not serialize frontmatter: {}", e),
                })?;
            format!("---\n{}\n---\n{}", yaml.trim_end(), body).into_bytes()
        } else {
            raw.as_bytes().to_vec()
        };
        std::fs::write(hub_skill_dir.join("SKILL.md"), &bytes_to_write).map_err(|source| {
            QuayError::Io {
                path: hub_skill_dir.display().to_string(),
                source,
            }
        })?;
        let skill_md_sha = sha256_of_bytes(&bytes_to_write);

        // 7. Copy the whole skill tree (including nested dirs) into the hub clone.
        // `skill_files` is reused below to populate registry.json `files`, so the
        // copied set and the indexed set are guaranteed identical.
        let skill_files = crate::skill_files::collect_skill_files(&local_skill_dir)?;
        for rel in &skill_files {
            if rel == "SKILL.md" {
                continue; // already written above (possibly with bumped frontmatter)
            }
            let from = local_skill_dir.join(rel);
            let to = hub_skill_dir.join(rel);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|source| QuayError::Io {
                    path: parent.display().to_string(),
                    source,
                })?;
            }
            std::fs::copy(&from, &to).map_err(|source| QuayError::Io {
                path: to.display().to_string(),
                source,
            })?;
        }

        // 7.5. Update registry.json so consumers (`quay search`, Browse,
        // `quay add`) can find this skill. Best-effort: a malformed existing
        // registry.json is replaced with a fresh one.
        let content_hash = crate::lock_hash::folder_hash(&hub_skill_dir)?;
        update_hub_registry(
            &hub_clone,
            &remote_name,
            skill_name,
            &manifest,
            &skill_md_sha,
            &skill_files,
            &content_hash,
        )?;

        // 8 & 9. Mode-aware branch + commit + push (+ optional PR).
        let result = match effective_mode {
            PushMode::Pr => self.finish_push_pr(
                &remote_name,
                skill_name,
                is_new_skill,
                &manifest,
                &hub_clone,
            )?,
            PushMode::Direct => self.finish_push_direct(
                &remote_name,
                skill_name,
                &manifest,
                &hub_clone,
                effective_direct_branch,
            )?,
        };

        // Best-effort push-log append — note pr_url is "" for direct.
        if let Some(config_dir) = &self.config_dir {
            let abs_project = self
                .project_root
                .canonicalize()
                .unwrap_or_else(|_| self.project_root.clone());
            if let Err(e) = PushLog::append(
                config_dir,
                PushRecord {
                    name: skill_name.to_string(),
                    remote: result.remote.clone(),
                    branch: result.branch.clone(),
                    pr_url: result
                        .pr
                        .as_ref()
                        .map(|p| p.url.clone())
                        .unwrap_or_default(),
                    pushed_at: chrono::Utc::now().to_rfc3339(),
                    commit_sha: Some(result.commit_sha.clone()),
                    project_path: Some(abs_project),
                },
            ) {
                eprintln!("warning: failed to write push-log: {}; push succeeded", e);
            }
        }

        Ok(result)
    }

    fn finish_push_pr(
        &self,
        remote_name: &str,
        skill_name: &str,
        is_new_skill: bool,
        manifest: &SkillManifest,
        hub_clone: &Path,
    ) -> Result<PushResult> {
        let branch = format!("quay/{}-{}", skill_name, manifest.version);
        self.git.checkout_new_branch(hub_clone, &branch)?;
        self.git.add_all(hub_clone)?;
        let (author_name, author_email) = self.author_identity()?;
        let did_commit = self.git.commit(
            hub_clone,
            &commit_message(skill_name, manifest),
            &author_name,
            &author_email,
        )?;
        if !did_commit {
            return Err(QuayError::ConfigValidation(format!(
                "no changes to push for {} (working tree was clean after copy)",
                skill_name
            )));
        }
        self.git.push(hub_clone, "origin", &branch)?;
        let commit_sha = self.git.head_sha(hub_clone)?;

        let title = format!(
            "{}: {} {}",
            skill_name,
            if is_new_skill { "add" } else { "update" },
            manifest.version
        );
        let body = pr_body(skill_name, manifest);
        let pr = self.opener.open_pr(hub_clone, &branch, &title, &body)?;

        Ok(PushResult {
            remote: remote_name.to_string(),
            branch,
            version: manifest.version.clone(),
            pr: Some(pr),
            commit_sha,
        })
    }

    fn finish_push_direct(
        &self,
        remote_name: &str,
        skill_name: &str,
        manifest: &SkillManifest,
        hub_clone: &Path,
        direct_branch: Option<&str>,
    ) -> Result<PushResult> {
        // The freshly-cloned repo is already on its default branch — read it back.
        let default_branch = self.git.current_branch(hub_clone)?;

        // If a specific branch was requested, switch to it (creating it from
        // the current default branch if it does not exist yet).
        let push_branch = if let Some(branch) = direct_branch {
            self.git.checkout_new_branch(hub_clone, branch)?;
            branch.to_string()
        } else {
            default_branch.clone()
        };

        self.git.add_all(hub_clone)?;
        let (author_name, author_email) = self.author_identity()?;
        let did_commit = self.git.commit(
            hub_clone,
            &commit_message(skill_name, manifest),
            &author_name,
            &author_email,
        )?;
        if !did_commit {
            return Err(QuayError::ConfigValidation(format!(
                "no changes to push for {} (working tree was clean after copy)",
                skill_name
            )));
        }

        self.git
            .push(hub_clone, "origin", &push_branch)
            .map_err(|e| {
                QuayError::ConfigValidation(format!(
                    "direct push to '{}' failed: {}; if the branch is protected, set this remote's push_mode = pr",
                    push_branch, e
                ))
            })?;
        let commit_sha = self.git.head_sha(hub_clone)?;

        Ok(PushResult {
            remote: remote_name.to_string(),
            branch: push_branch,
            version: manifest.version.clone(),
            pr: None,
            commit_sha,
        })
    }

    fn author_identity(&self) -> Result<(String, String)> {
        if let Some((n, e)) = &self.author {
            return Ok((n.clone(), e.clone()));
        }
        let name = self
            .config
            .user
            .name
            .clone()
            .unwrap_or_else(|| "Quay User".into());
        let email = self.config.user.email.clone().ok_or_else(|| {
            QuayError::ConfigValidation(
                "no commit author email — set [user] email in config or pass --author".into(),
            )
        })?;
        Ok((name, email))
    }
}

fn commit_message(skill_name: &str, manifest: &SkillManifest) -> String {
    format!(
        "{} {} via quay\n\n{}",
        skill_name, manifest.version, manifest.description
    )
}

/// If `raw` starts with a `---\n…\n---\n` YAML frontmatter block, return
/// just the body that follows it. Otherwise return `None`.
fn strip_frontmatter(raw: &str) -> Option<&str> {
    let trimmed = raw.trim_start_matches('\u{feff}');
    let rest = trimmed.strip_prefix("---\n")?;
    rest.split_once("\n---\n").map(|(_yaml, body)| body)
}

fn sha256_of_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Read `<hub_clone>/registry.json` (or start fresh if missing/malformed),
/// add or update the entry for `skill_name` from `manifest` + `sha`, then
/// write it back.
fn update_hub_registry(
    hub_clone: &Path,
    hub_name: &str,
    skill_name: &str,
    manifest: &SkillManifest,
    skill_md_sha: &str,
    files: &[String],
    content_hash: &str,
) -> Result<()> {
    use crate::registry::{Registry, RegistryEntry};
    use std::collections::BTreeMap;

    let path = hub_clone.join("registry.json");
    let mut registry = if let Ok(text) = std::fs::read_to_string(&path) {
        Registry::parse(&text).unwrap_or_else(|_| Registry {
            hub: hub_name.to_string(),
            generated_at: chrono::Utc::now().to_rfc3339(),
            schema_version: 1,
            skills: BTreeMap::new(),
        })
    } else {
        Registry {
            hub: hub_name.to_string(),
            generated_at: chrono::Utc::now().to_rfc3339(),
            schema_version: 1,
            skills: BTreeMap::new(),
        }
    };

    let entry = RegistryEntry {
        version: manifest.version.clone(),
        description: manifest.description.clone(),
        category: manifest.category.clone(),
        tags: manifest.tags.clone(),
        path: format!("skills/{}", skill_name),
        sha: skill_md_sha.to_string(),
        files: files.to_vec(),
        source_format: manifest.source_format,
        content_hash: content_hash.to_string(),
    };
    registry.skills.insert(skill_name.to_string(), entry);
    registry.generated_at = chrono::Utc::now().to_rfc3339();

    let body = serde_json::to_string_pretty(&registry).map_err(|e| QuayError::InvalidRegistry {
        reason: format!("serialise registry: {}", e),
    })?;
    std::fs::write(&path, body).map_err(|source| QuayError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

fn pr_body(skill_name: &str, manifest: &SkillManifest) -> String {
    let category = manifest.category.as_deref().unwrap_or("uncategorized");
    let tags = if manifest.tags.is_empty() {
        "—".to_string()
    } else {
        manifest.tags.join(", ")
    };
    format!(
        "Pushed via `quay push`.\n\n- skill: `{}`\n- version: `{}`\n- category: `{}`\n- tags: {}\n\n{}\n",
        skill_name, manifest.version, category, tags, manifest.description
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RemoteConfig;
    use crate::provider::FakeOpener;
    use std::cell::RefCell;

    /// `FakeGit` records every call and serves a working clone of an empty repo for `clone()`.
    ///
    /// Set `seed_skill` to `Some((name, content))` to pre-populate
    /// `<dest>/skills/<name>/SKILL.md` on every `clone()` call, simulating a
    /// hub that already contains a previous version of the skill.
    struct FakeGit {
        clones: RefCell<Vec<(String, PathBuf)>>,
        branches: RefCell<Vec<(PathBuf, String)>>,
        commits: RefCell<Vec<(PathBuf, String)>>,
        pushes: RefCell<Vec<(PathBuf, String, String)>>,
        /// If set, clone() seeds `<dest>/skills/<name>/SKILL.md` with this content.
        seed_skill: Option<(String, String)>,
        /// If set, the next `push()` call returns this error message and clears the flag.
        push_failure: RefCell<Option<String>>,
    }

    impl Default for FakeGit {
        fn default() -> Self {
            Self::new()
        }
    }

    impl FakeGit {
        fn new() -> Self {
            Self {
                clones: RefCell::new(Vec::new()),
                branches: RefCell::new(Vec::new()),
                commits: RefCell::new(Vec::new()),
                pushes: RefCell::new(Vec::new()),
                seed_skill: None,
                push_failure: RefCell::new(None),
            }
        }

        fn with_seed(name: impl Into<String>, content: impl Into<String>) -> Self {
            Self {
                clones: RefCell::new(Vec::new()),
                branches: RefCell::new(Vec::new()),
                commits: RefCell::new(Vec::new()),
                pushes: RefCell::new(Vec::new()),
                seed_skill: Some((name.into(), content.into())),
                push_failure: RefCell::new(None),
            }
        }

        /// Constructs a `FakeGit` that causes the next `push()` call to fail with
        /// the given message.
        fn with_push_failure(message: &str) -> Self {
            Self {
                clones: RefCell::new(Vec::new()),
                branches: RefCell::new(Vec::new()),
                commits: RefCell::new(Vec::new()),
                pushes: RefCell::new(Vec::new()),
                seed_skill: None,
                push_failure: RefCell::new(Some(message.to_string())),
            }
        }

        /// Returns the branch name from the last recorded `push()` call, if any.
        fn last_pushed_branch(&self) -> Option<String> {
            self.pushes
                .borrow()
                .last()
                .map(|(_, _, branch)| branch.clone())
        }
    }

    impl GitClient for FakeGit {
        fn clone(&self, url: &str, dest: &Path, _branch: Option<&str>) -> Result<()> {
            self.clones
                .borrow_mut()
                .push((url.into(), dest.to_path_buf()));
            std::fs::create_dir_all(dest).unwrap();
            if let Some((skill_name, content)) = &self.seed_skill {
                let skill_dir = dest.join("skills").join(skill_name);
                std::fs::create_dir_all(&skill_dir).unwrap();
                std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
            }
            Ok(())
        }

        fn checkout_new_branch(&self, repo: &Path, branch: &str) -> Result<()> {
            self.branches
                .borrow_mut()
                .push((repo.to_path_buf(), branch.into()));
            Ok(())
        }

        fn add_all(&self, _repo: &Path) -> Result<()> {
            Ok(())
        }

        fn commit(&self, repo: &Path, message: &str, _name: &str, _email: &str) -> Result<bool> {
            self.commits
                .borrow_mut()
                .push((repo.to_path_buf(), message.into()));
            Ok(true)
        }

        fn push(&self, repo: &Path, remote: &str, branch: &str) -> Result<String> {
            self.pushes
                .borrow_mut()
                .push((repo.to_path_buf(), remote.into(), branch.into()));
            if let Some(msg) = self.push_failure.borrow_mut().take() {
                return Err(QuayError::ConfigValidation(msg));
            }
            Ok("https://example.test/foo/bar.git".into())
        }

        fn remote_url(&self, _repo: &Path, _remote: &str) -> Result<String> {
            Ok("https://example.test/foo/bar.git".into())
        }

        fn current_branch(&self, _repo: &Path) -> Result<String> {
            Ok("main".to_string())
        }

        fn head_sha(&self, _repo: &Path) -> Result<String> {
            Ok("0000000000000000000000000000000000000000".to_string())
        }
    }

    /// Opener that captures the PR title so tests can assert on "add" vs "update".
    #[derive(Default)]
    struct TitleCapturingOpener {
        pub title: RefCell<String>,
    }

    impl PrOpener for TitleCapturingOpener {
        fn open_pr(&self, _repo: &Path, branch: &str, title: &str, _body: &str) -> Result<PrInfo> {
            *self.title.borrow_mut() = title.to_string();
            Ok(PrInfo {
                url: format!("https://example.test/{}", branch),
                auto_created: false,
            })
        }
    }

    fn make_local_skill(root: &Path, name: &str, body: &str) {
        let dir = root.join(".agents/skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), body).unwrap();
    }

    fn make_config() -> Config {
        let mut cfg = Config::default();
        cfg.user.email = Some("alice@example.com".into());
        cfg.user.name = Some("Alice".into());
        cfg.remotes.insert(
            "h".into(),
            RemoteConfig {
                url: "https://github.com/foo/bar.git".into(),
                default: true,
                provider: None,
                push_mode: crate::config::PushMode::default(),
                direct_branch: None,
            },
        );
        cfg
    }

    #[test]
    fn push_runs_full_pipeline() {
        let project = assert_fs::TempDir::new().unwrap();
        let clone_root = assert_fs::TempDir::new().unwrap();
        make_local_skill(
            project.path(),
            "csv-parse",
            "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 0.1.0\n---\nbody\n",
        );
        let cfg = make_config();
        let git = FakeGit::new();
        let opener = FakeOpener;

        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };
        let result = pusher
            .push(
                "csv-parse",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None,
            )
            .unwrap();
        assert_eq!(result.remote, "h");
        assert_eq!(result.branch, "quay/csv-parse-0.1.0");
        assert_eq!(result.version, "0.1.0");
        assert!(result.pr.as_ref().unwrap().url.contains("csv-parse"));
        // Verify the call sequence happened.
        assert_eq!(git.clones.borrow().len(), 1);
        assert_eq!(git.branches.borrow().len(), 1);
        assert_eq!(git.commits.borrow().len(), 1);
        assert_eq!(git.pushes.borrow().len(), 1);
    }

    #[test]
    fn push_bumps_version_per_kind() {
        let project = assert_fs::TempDir::new().unwrap();
        let clone_root = assert_fs::TempDir::new().unwrap();
        make_local_skill(
            project.path(),
            "csv-parse",
            "---\nname: csv-parse\ndescription: x.\nversion: 1.2.3\n---\nbody\n",
        );
        let cfg = make_config();
        let git = FakeGit::new();
        let opener = FakeOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };

        let r = pusher
            .push(
                "csv-parse",
                None,
                BumpKind::Patch,
                clone_root.path(),
                None,
                None,
            )
            .unwrap();
        assert_eq!(r.version, "1.2.4");

        // Need a fresh clone-root each call to avoid colliding hub-csv-parse dir.
        let cr2 = assert_fs::TempDir::new().unwrap();
        let r = pusher
            .push("csv-parse", None, BumpKind::Minor, cr2.path(), None, None)
            .unwrap();
        assert_eq!(r.version, "1.3.0");

        let cr3 = assert_fs::TempDir::new().unwrap();
        let r = pusher
            .push("csv-parse", None, BumpKind::Major, cr3.path(), None, None)
            .unwrap();
        assert_eq!(r.version, "2.0.0");
    }

    #[test]
    fn push_errors_when_skill_missing_locally() {
        let project = assert_fs::TempDir::new().unwrap();
        let clone_root = assert_fs::TempDir::new().unwrap();
        let cfg = make_config();
        let git = FakeGit::new();
        let opener = FakeOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };
        let err = pusher
            .push(
                "does-not-exist",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None,
            )
            .unwrap_err();
        assert!(matches!(err, QuayError::SkillNotFound { .. }));
    }

    #[test]
    fn push_marks_existing_skill_as_update_in_pr_title() {
        let project = assert_fs::TempDir::new().unwrap();
        let clone_root = assert_fs::TempDir::new().unwrap();
        make_local_skill(
            project.path(),
            "csv-parse",
            "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.2.3\n---\nbody\n",
        );
        let cfg = make_config();
        // Seed the hub clone with an existing SKILL.md so is_new_skill is false.
        let git = FakeGit::with_seed(
            "csv-parse",
            "---\nname: csv-parse\ndescription: old.\nversion: 0.1.0\n---\nold\n",
        );
        let opener = TitleCapturingOpener::default();
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };
        pusher
            .push(
                "csv-parse",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None,
            )
            .unwrap();
        let title = opener.title.borrow().clone();
        assert!(
            title.contains("update"),
            "expected 'update', got: {}",
            title
        );
    }

    #[test]
    fn push_marks_new_skill_as_add_in_pr_title() {
        let project = assert_fs::TempDir::new().unwrap();
        let clone_root = assert_fs::TempDir::new().unwrap();
        make_local_skill(
            project.path(),
            "csv-parse",
            "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.2.3\n---\nbody\n",
        );
        let cfg = make_config();
        // No seed: hub has no existing skill, so is_new_skill is true.
        let git = FakeGit::new();
        let opener = TitleCapturingOpener::default();
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };
        pusher
            .push(
                "csv-parse",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None,
            )
            .unwrap();
        let title = opener.title.borrow().clone();
        assert!(title.contains("add"), "expected 'add', got: {}", title);
    }

    #[test]
    fn push_errors_when_no_default_and_no_pinned_remote() {
        let project = assert_fs::TempDir::new().unwrap();
        let clone_root = assert_fs::TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.user.email = Some("a@e".into());
        // Add a remote but DON'T mark it default.
        cfg.remotes.insert(
            "h".into(),
            RemoteConfig {
                url: "u".into(),
                default: false,
                provider: None,
                push_mode: crate::config::PushMode::default(),
                direct_branch: None,
            },
        );
        make_local_skill(
            project.path(),
            "x",
            "---\nname: x\ndescription: y\nversion: 0.1.0\n---\nbody\n",
        );
        let git = FakeGit::new();
        let opener = FakeOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };
        let err = pusher
            .push(
                "x",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None,
            )
            .unwrap_err();
        assert!(matches!(err, QuayError::ConfigValidation(_)));
    }

    #[test]
    fn push_copies_nested_subdir_files_into_hub_clone() {
        let project = assert_fs::TempDir::new().unwrap();
        let clone_root = assert_fs::TempDir::new().unwrap();
        // Local skill with a nested script + agent file.
        let dir = project.path().join(".agents/skills/nested");
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: nested\ndescription: n\nversion: 0.1.0\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(dir.join("scripts/sync.mjs"), "code").unwrap();
        std::fs::write(dir.join("agents/openai.yaml"), "cfg").unwrap();

        let cfg = make_config();
        let git = FakeGit::new();
        let opener = FakeOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };
        pusher
            .push(
                "nested",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None,
            )
            .unwrap();

        let hub = clone_root.path().join("hub-nested/skills/nested");
        assert_eq!(
            std::fs::read_to_string(hub.join("scripts/sync.mjs")).unwrap(),
            "code"
        );
        assert_eq!(
            std::fs::read_to_string(hub.join("agents/openai.yaml")).unwrap(),
            "cfg"
        );
    }

    #[test]
    fn push_writes_nested_files_into_hub_registry() {
        let project = assert_fs::TempDir::new().unwrap();
        let clone_root = assert_fs::TempDir::new().unwrap();
        let dir = project.path().join(".agents/skills/nested");
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: nested\ndescription: n\nversion: 0.1.0\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(dir.join("scripts/sync.mjs"), "code").unwrap();

        let cfg = make_config();
        let git = FakeGit::new();
        let opener = FakeOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };
        pusher
            .push(
                "nested",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None,
            )
            .unwrap();

        let registry_json = clone_root.path().join("hub-nested/registry.json");
        let text = std::fs::read_to_string(&registry_json).unwrap();
        let reg = crate::registry::Registry::parse(&text).unwrap();
        assert_eq!(
            reg.entry("nested").unwrap().files,
            vec!["SKILL.md".to_string(), "scripts/sync.mjs".to_string()]
        );
    }

    #[test]
    fn push_works_on_skill_without_frontmatter() {
        let project = assert_fs::TempDir::new().unwrap();
        let config_dir = tempfile::TempDir::new().unwrap();
        let clone_root = assert_fs::TempDir::new().unwrap();
        let freestyle_body = "## Notes\n\nThis is a freestyle skill with no frontmatter.\n";
        make_local_skill(project.path(), "free-skill", freestyle_body);
        let cfg = make_config();
        let git = FakeGit::new();
        let opener = FakeOpener;

        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: Some(config_dir.path().to_path_buf()),
            author: None,
        };

        // 1. Push returns Ok.
        let result = pusher
            .push(
                "free-skill",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None,
            )
            .unwrap();
        assert_eq!(result.remote, "h");
        assert!(result.pr.as_ref().unwrap().url.contains("free-skill"));

        // 2. User-level push-log exists with one record matching the skill name.
        let log = PushLog::load(config_dir.path(), Some(project.path())).unwrap();
        assert_eq!(log.records.len(), 1);
        assert_eq!(log.records[0].name, "free-skill");
        assert_eq!(log.records[0].pr_url, result.pr.as_ref().unwrap().url);

        // 3. The file written into the hub clone equals the raw bytes (no frontmatter
        //    re-emission, since bump=AsWritten and format=Freestyle).
        let hub_skill_md = clone_root
            .path()
            .join("hub-free-skill/skills/free-skill/SKILL.md");
        let written = std::fs::read_to_string(&hub_skill_md).unwrap();
        assert_eq!(written, freestyle_body);
    }

    #[test]
    fn push_errors_on_bump_for_non_frontmatter_skill() {
        let project = assert_fs::TempDir::new().unwrap();
        let clone_root = assert_fs::TempDir::new().unwrap();
        make_local_skill(
            project.path(),
            "slash-skill",
            "# /slash-skill\n\nDoes something.\n",
        );
        let cfg = make_config();
        let git = FakeGit::new();
        let opener = FakeOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };
        let err = pusher
            .push(
                "slash-skill",
                None,
                BumpKind::Patch,
                clone_root.path(),
                None,
                None,
            )
            .unwrap_err();
        assert!(matches!(err, QuayError::ConfigValidation(_)));
    }

    #[test]
    fn push_writes_push_log_after_frontmatter_skill() {
        let project = assert_fs::TempDir::new().unwrap();
        let config_dir = tempfile::TempDir::new().unwrap();
        let clone_root = assert_fs::TempDir::new().unwrap();
        make_local_skill(
            project.path(),
            "csv-parse",
            "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 0.1.0\n---\nbody\n",
        );
        let cfg = make_config();
        let git = FakeGit::new();
        let opener = FakeOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: Some(config_dir.path().to_path_buf()),
            author: None,
        };
        let result = pusher
            .push(
                "csv-parse",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None,
            )
            .unwrap();

        let log = PushLog::load(config_dir.path(), Some(project.path())).unwrap();
        assert_eq!(log.records.len(), 1);
        assert_eq!(log.records[0].name, "csv-parse");
        assert_eq!(log.records[0].remote, "h");
        assert_eq!(log.records[0].pr_url, result.pr.as_ref().unwrap().url);
    }

    // ── direct-mode tests ────────────────────────────────────────────────────

    use crate::config::PushMode;

    /// A `PrOpener` that panics if invoked — proves direct mode never opens a PR.
    struct PanickingPrOpener;

    impl PrOpener for PanickingPrOpener {
        fn open_pr(
            &self,
            _repo: &Path,
            _branch: &str,
            _title: &str,
            _body: &str,
        ) -> Result<PrInfo> {
            panic!("direct mode must not open a PR");
        }
    }

    fn make_direct_config() -> crate::config::Config {
        let mut cfg = crate::config::Config::default();
        cfg.user.email = Some("dev@example.com".into());
        cfg.remotes.insert(
            "hub".into(),
            crate::config::RemoteConfig {
                url: "git@example:o/r.git".into(),
                default: true,
                provider: None,
                push_mode: PushMode::Direct,
                direct_branch: None,
            },
        );
        cfg
    }

    #[test]
    fn direct_mode_pushes_to_default_branch_and_skips_pr() {
        let project = tempfile::TempDir::new().unwrap();
        make_local_skill(
            project.path(),
            "foo",
            "---\nname: foo\ndescription: f\nversion: 0.1.0\n---\nbody\n",
        );

        let cfg = make_direct_config();
        let git = FakeGit::default();
        let opener = PanickingPrOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };

        let clone_root = tempfile::TempDir::new().unwrap();
        let result = pusher
            .push(
                "foo",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None,
            )
            .unwrap();

        assert!(result.pr.is_none(), "direct mode must not produce a PR");
        assert_eq!(
            result.branch, "main",
            "FakeGit::current_branch returns 'main'"
        );
        assert_eq!(result.commit_sha.len(), 40);
        assert_eq!(
            git.last_pushed_branch().as_deref(),
            Some("main"),
            "push() must target the default branch"
        );
    }

    #[test]
    fn direct_mode_records_commit_sha_in_push_log() {
        let project = tempfile::TempDir::new().unwrap();
        let config_dir = tempfile::TempDir::new().unwrap();
        make_local_skill(
            project.path(),
            "foo",
            "---\nname: foo\ndescription: f\nversion: 0.1.0\n---\nbody\n",
        );

        let cfg = make_direct_config();
        let git = FakeGit::default();
        let opener = PanickingPrOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: Some(config_dir.path().to_path_buf()),
            author: None,
        };
        let clone_root = tempfile::TempDir::new().unwrap();
        let _ = pusher
            .push(
                "foo",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None,
            )
            .unwrap();

        let log = crate::push_log::PushLog::load(config_dir.path(), Some(project.path())).unwrap();
        assert_eq!(log.records.len(), 1);
        assert!(
            log.records[0].pr_url.is_empty(),
            "direct mode pr_url must be empty"
        );
        assert!(
            log.records[0].commit_sha.is_some(),
            "commit_sha must be recorded"
        );
    }

    #[test]
    fn push_mode_override_some_direct_overrides_pr_remote_default() {
        let project = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(project.path().join(".agents/skills/foo")).unwrap();
        std::fs::write(
            project.path().join(".agents/skills/foo/SKILL.md"),
            "---\nname: foo\ndescription: f\nversion: 0.1.0\n---\nbody\n",
        )
        .unwrap();

        let mut cfg = crate::config::Config::default();
        cfg.user.email = Some("dev@example.com".into());
        cfg.remotes.insert(
            "hub".into(),
            crate::config::RemoteConfig {
                url: "git@example:o/r.git".into(),
                default: true,
                provider: None,
                push_mode: crate::config::PushMode::Pr, // remote default is PR
                direct_branch: None,
            },
        );

        let git = FakeGit::default();
        let opener = PanickingPrOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };
        let clone_root = tempfile::TempDir::new().unwrap();
        let result = pusher
            .push(
                "foo",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                Some(crate::config::PushMode::Direct),
                None,
            )
            .unwrap();
        assert!(
            result.pr.is_none(),
            "override Direct must skip PR even when remote default is Pr"
        );
    }

    #[test]
    fn direct_mode_surfaces_protected_branch_error() {
        let project = tempfile::TempDir::new().unwrap();
        make_local_skill(
            project.path(),
            "foo",
            "---\nname: foo\ndescription: f\nversion: 0.1.0\n---\nbody\n",
        );

        let cfg = make_direct_config();
        let git = FakeGit::with_push_failure(
            "remote: error: GH006: Protected branch update failed for refs/heads/main",
        );
        let opener = PanickingPrOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };
        let clone_root = tempfile::TempDir::new().unwrap();
        let err = pusher
            .push(
                "foo",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None,
            )
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("direct push"),
            "expected 'direct push' in error: {msg}"
        );
        assert!(
            msg.contains("push_mode = pr"),
            "expected 'push_mode = pr' hint in error: {msg}"
        );
    }

    // ── direct_branch tests ──────────────────────────────────────────────────

    /// Helper that creates a direct-mode config with `direct_branch` set.
    fn make_direct_config_with_branch(branch: &str) -> crate::config::Config {
        let mut cfg = crate::config::Config::default();
        cfg.user.email = Some("dev@example.com".into());
        cfg.remotes.insert(
            "hub".into(),
            crate::config::RemoteConfig {
                url: "git@example:o/r.git".into(),
                default: true,
                provider: None,
                push_mode: PushMode::Direct,
                direct_branch: Some(branch.to_string()),
            },
        );
        cfg
    }

    #[test]
    fn direct_mode_with_config_direct_branch_pushes_to_that_branch() {
        let project = tempfile::TempDir::new().unwrap();
        make_local_skill(
            project.path(),
            "foo",
            "---\nname: foo\ndescription: f\nversion: 0.1.0\n---\nbody\n",
        );

        let cfg = make_direct_config_with_branch("develop");
        let git = FakeGit::default();
        let opener = PanickingPrOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };

        let clone_root = tempfile::TempDir::new().unwrap();
        let result = pusher
            .push(
                "foo",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None, // no per-invocation override; should use config
            )
            .unwrap();

        assert!(result.pr.is_none(), "direct mode must not open a PR");
        assert_eq!(
            result.branch, "develop",
            "result.branch must be the configured direct_branch"
        );
        assert_eq!(
            git.last_pushed_branch().as_deref(),
            Some("develop"),
            "git push must target 'develop'"
        );
        // checkout_new_branch must have been called to switch to the branch.
        let branches = git.branches.borrow();
        assert!(
            branches.iter().any(|(_, b)| b == "develop"),
            "checkout_new_branch must be called with 'develop'"
        );
    }

    #[test]
    fn direct_branch_override_wins_over_config() {
        let project = tempfile::TempDir::new().unwrap();
        make_local_skill(
            project.path(),
            "foo",
            "---\nname: foo\ndescription: f\nversion: 0.1.0\n---\nbody\n",
        );

        // Config says "develop" but the per-invocation override is "skills".
        let cfg = make_direct_config_with_branch("develop");
        let git = FakeGit::default();
        let opener = PanickingPrOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };

        let clone_root = tempfile::TempDir::new().unwrap();
        let result = pusher
            .push(
                "foo",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                Some("skills"), // per-invocation override
            )
            .unwrap();

        assert_eq!(
            result.branch, "skills",
            "per-invocation override must win over config direct_branch"
        );
        assert_eq!(
            git.last_pushed_branch().as_deref(),
            Some("skills"),
            "git push must target 'skills'"
        );
    }

    #[test]
    fn direct_branch_none_falls_back_to_default_branch() {
        let project = tempfile::TempDir::new().unwrap();
        make_local_skill(
            project.path(),
            "foo",
            "---\nname: foo\ndescription: f\nversion: 0.1.0\n---\nbody\n",
        );

        // Config has no direct_branch set.
        let cfg = make_direct_config();
        let git = FakeGit::default();
        let opener = PanickingPrOpener;
        let pusher = SkillPusher {
            config: &cfg,
            git: &git,
            opener: &opener,
            project_root: project.path().to_path_buf(),
            config_dir: None,
            author: None,
        };

        let clone_root = tempfile::TempDir::new().unwrap();
        let result = pusher
            .push(
                "foo",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
                None,
                None, // no override
            )
            .unwrap();

        assert_eq!(
            result.branch, "main",
            "None direct_branch must fall back to default branch (FakeGit returns 'main')"
        );
        assert_eq!(
            git.last_pushed_branch().as_deref(),
            Some("main"),
            "git push must target default branch"
        );
        // checkout_new_branch must NOT have been called (no branch switch).
        assert!(
            git.branches.borrow().is_empty(),
            "checkout_new_branch must NOT be called when direct_branch is None"
        );
    }
}
