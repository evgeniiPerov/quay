//! Orchestrates the local-skill → hub-PR pipeline.
//!
//! [`SkillPusher`] is generic over [`GitClient`] and [`PrOpener`] so tests can
//! inject fakes without spawning real git processes or making network calls.

use crate::config::Config;
use crate::error::{QuayError, Result};
use crate::git::GitClient;
use crate::manifest::{parse_skill, SkillManifest};
use crate::provider::{PrInfo, PrOpener};
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
    pub pr: PrInfo,
}

/// Drives the local-skill → hub-PR pipeline.
pub struct SkillPusher<'a, G: GitClient, P: PrOpener> {
    pub config: &'a Config,
    pub git: &'a G,
    pub opener: &'a P,
    pub project_root: PathBuf,
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
        let md_text = std::fs::read_to_string(&local_md_path).map_err(|source| QuayError::Io {
            path: local_md_path.display().to_string(),
            source,
        })?;
        let (mut manifest, body) = parse_skill(&md_text, &local_md_path.display().to_string())?;

        // 3. Apply version bump (in memory; written on commit).
        match bump {
            BumpKind::AsWritten => {}
            BumpKind::Patch | BumpKind::Minor | BumpKind::Major => {
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

        // 4. Clone the hub.
        let hub_clone = clone_dest_root.join(format!("hub-{}", skill_name));
        if hub_clone.exists() {
            std::fs::remove_dir_all(&hub_clone).map_err(|source| QuayError::Io {
                path: hub_clone.display().to_string(),
                source,
            })?;
        }
        self.git.clone(&remote_cfg.url, &hub_clone, None)?;

        // 5. Make sure target dir exists in the hub clone (default to flat layout if new).
        let hub_skill_dir = hub_clone.join("skills").join(skill_name);
        std::fs::create_dir_all(&hub_skill_dir).map_err(|source| QuayError::Io {
            path: hub_skill_dir.display().to_string(),
            source,
        })?;

        // 6. Write the (possibly bumped) manifest + body back to disk in the hub clone.
        let new_md = format!(
            "---\n{}\n---\n{}",
            serde_yaml::to_string(&manifest)
                .map_err(|e| QuayError::InvalidFrontmatter {
                    path: hub_skill_dir.display().to_string(),
                    reason: format!("could not serialize frontmatter: {}", e),
                })?
                .trim_end(),
            body
        );
        std::fs::write(hub_skill_dir.join("SKILL.md"), new_md).map_err(|source| QuayError::Io {
            path: hub_skill_dir.display().to_string(),
            source,
        })?;

        // 7. Copy any extra files alongside SKILL.md from local.
        for entry in std::fs::read_dir(&local_skill_dir).map_err(|source| QuayError::Io {
            path: local_skill_dir.display().to_string(),
            source,
        })? {
            let entry = entry.map_err(|source| QuayError::Io {
                path: local_skill_dir.display().to_string(),
                source,
            })?;
            let name = entry.file_name();
            if name == "SKILL.md" {
                continue;
            }
            let from = entry.path();
            let to = hub_skill_dir.join(&name);
            if from.is_file() {
                std::fs::copy(&from, &to).map_err(|source| QuayError::Io {
                    path: to.display().to_string(),
                    source,
                })?;
            }
            // Subdirectories (resources/, scripts/) are not yet supported here — flag for Plan 4.
        }

        // 8. Branch / commit / push.
        let branch = format!("quay/{}-{}", skill_name, manifest.version);
        self.git.checkout_new_branch(&hub_clone, &branch)?;
        self.git.add_all(&hub_clone)?;
        let (author_name, author_email) = self.author_identity()?;
        let did_commit = self.git.commit(
            &hub_clone,
            &commit_message(skill_name, &manifest),
            &author_name,
            &author_email,
        )?;
        if !did_commit {
            return Err(QuayError::ConfigValidation(format!(
                "no changes to push for {} (working tree was clean after copy)",
                skill_name
            )));
        }
        self.git.push(&hub_clone, "origin", &branch)?;

        // 9. Open PR.
        let title = format!(
            "{}: {} {}",
            skill_name,
            if branch.contains("0.1.0") {
                "add"
            } else {
                "update"
            },
            manifest.version
        );
        let body = pr_body(skill_name, &manifest);
        let pr = self.opener.open_pr(&hub_clone, &branch, &title, &body)?;

        Ok(PushResult {
            remote: remote_name,
            branch,
            version: manifest.version,
            pr,
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
    struct FakeGit {
        clones: RefCell<Vec<(String, PathBuf)>>,
        branches: RefCell<Vec<(PathBuf, String)>>,
        commits: RefCell<Vec<(PathBuf, String)>>,
        pushes: RefCell<Vec<(PathBuf, String, String)>>,
    }

    impl FakeGit {
        fn new() -> Self {
            Self {
                clones: RefCell::new(Vec::new()),
                branches: RefCell::new(Vec::new()),
                commits: RefCell::new(Vec::new()),
                pushes: RefCell::new(Vec::new()),
            }
        }
    }

    impl GitClient for FakeGit {
        fn clone(&self, url: &str, dest: &Path, _branch: Option<&str>) -> Result<()> {
            self.clones
                .borrow_mut()
                .push((url.into(), dest.to_path_buf()));
            std::fs::create_dir_all(dest).unwrap();
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
            Ok("https://example.test/foo/bar.git".into())
        }

        fn remote_url(&self, _repo: &Path, _remote: &str) -> Result<String> {
            Ok("https://example.test/foo/bar.git".into())
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
            author: None,
        };
        let result = pusher
            .push("csv-parse", None, BumpKind::AsWritten, clone_root.path())
            .unwrap();
        assert_eq!(result.remote, "h");
        assert_eq!(result.branch, "quay/csv-parse-0.1.0");
        assert_eq!(result.version, "0.1.0");
        assert!(result.pr.url.contains("csv-parse"));
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
            author: None,
        };

        let r = pusher
            .push("csv-parse", None, BumpKind::Patch, clone_root.path())
            .unwrap();
        assert_eq!(r.version, "1.2.4");

        // Need a fresh clone-root each call to avoid colliding hub-csv-parse dir.
        let cr2 = assert_fs::TempDir::new().unwrap();
        let r = pusher
            .push("csv-parse", None, BumpKind::Minor, cr2.path())
            .unwrap();
        assert_eq!(r.version, "1.3.0");

        let cr3 = assert_fs::TempDir::new().unwrap();
        let r = pusher
            .push("csv-parse", None, BumpKind::Major, cr3.path())
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
            author: None,
        };
        let err = pusher
            .push(
                "does-not-exist",
                None,
                BumpKind::AsWritten,
                clone_root.path(),
            )
            .unwrap_err();
        assert!(matches!(err, QuayError::SkillNotFound { .. }));
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
            author: None,
        };
        let err = pusher
            .push("x", None, BumpKind::AsWritten, clone_root.path())
            .unwrap_err();
        assert!(matches!(err, QuayError::ConfigValidation(_)));
    }
}
