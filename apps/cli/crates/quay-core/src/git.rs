//! Git operations needed by `quay push`. Isolates the shell-out behind a trait
//! so tests can inject fakes without spawning real git processes.

use crate::error::{QuayError, Result};
use std::path::Path;
use std::process::Command;

/// Minimal git operations needed by `quay push`. Production impl shells out to `git`.
pub trait GitClient {
    /// Shallow-clone `url` into `dest`, optionally checking out `branch`.
    fn clone(&self, url: &str, dest: &Path, branch: Option<&str>) -> Result<()>;

    /// `git -C <repo> checkout -B <branch>` — create or reset the branch to current HEAD.
    fn checkout_new_branch(&self, repo: &Path, branch: &str) -> Result<()>;

    /// `git -C <repo> add -A`.
    fn add_all(&self, repo: &Path) -> Result<()>;

    /// Commit if there are staged changes. Returns `true` if a commit was created,
    /// `false` if the working tree was clean (nothing to commit).
    fn commit(
        &self,
        repo: &Path,
        message: &str,
        author_name: &str,
        author_email: &str,
    ) -> Result<bool>;

    /// `git -C <repo> push <remote> <branch>`. Returns the pushed remote URL on success.
    fn push(&self, repo: &Path, remote: &str, branch: &str) -> Result<String>;

    /// Read the URL of `origin` (or the named remote) — used to surface the right URL
    /// in user messages after a successful push.
    fn remote_url(&self, repo: &Path, remote: &str) -> Result<String>;
}

/// Production [`GitClient`] that spawns the user's `git` binary.
pub struct GitShellClient;

impl Default for GitShellClient {
    fn default() -> Self {
        Self
    }
}

fn run(cmd: &mut Command, where_for_error: &str) -> Result<std::process::Output> {
    let output = cmd.output().map_err(|source| QuayError::Io {
        path: where_for_error.into(),
        source,
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(QuayError::ConfigValidation(format!(
            "git command failed at {}: {}",
            where_for_error,
            stderr.trim()
        )));
    }
    Ok(output)
}

impl GitClient for GitShellClient {
    fn clone(&self, url: &str, dest: &Path, branch: Option<&str>) -> Result<()> {
        let mut cmd = Command::new("git");
        cmd.arg("clone").arg("--depth=1");
        if let Some(b) = branch {
            cmd.arg("--branch").arg(b);
        }
        cmd.arg(url).arg(dest);
        run(&mut cmd, &format!("clone {}", url))?;
        Ok(())
    }

    fn checkout_new_branch(&self, repo: &Path, branch: &str) -> Result<()> {
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(repo)
            .arg("checkout")
            .arg("-B")
            .arg(branch);
        run(&mut cmd, &format!("checkout -B {}", branch))?;
        Ok(())
    }

    fn add_all(&self, repo: &Path) -> Result<()> {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(repo).arg("add").arg("-A");
        run(&mut cmd, "git add -A")?;
        Ok(())
    }

    fn commit(
        &self,
        repo: &Path,
        message: &str,
        author_name: &str,
        author_email: &str,
    ) -> Result<bool> {
        // Detect whether anything is staged first.
        let mut diff = Command::new("git");
        diff.arg("-C")
            .arg(repo)
            .arg("diff")
            .arg("--cached")
            .arg("--quiet");
        let status = diff.status().map_err(|source| QuayError::Io {
            path: "git diff --cached".into(),
            source,
        })?;
        if status.success() {
            // exit 0 = no staged changes
            return Ok(false);
        }

        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(repo)
            .arg("-c")
            .arg(format!("user.name={}", author_name))
            .arg("-c")
            .arg(format!("user.email={}", author_email))
            .arg("commit")
            .arg("-m")
            .arg(message);
        run(&mut cmd, "git commit")?;
        Ok(true)
    }

    fn push(&self, repo: &Path, remote: &str, branch: &str) -> Result<String> {
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(repo)
            .arg("push")
            .arg("--set-upstream")
            .arg(remote)
            .arg(branch);
        run(&mut cmd, &format!("git push {} {}", remote, branch))?;
        self.remote_url(repo, remote)
    }

    fn remote_url(&self, repo: &Path, remote: &str) -> Result<String> {
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(repo)
            .arg("remote")
            .arg("get-url")
            .arg(remote);
        let out = run(&mut cmd, &format!("git remote get-url {}", remote))?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::prelude::*;
    use std::path::PathBuf;

    /// Initialize a bare repo at `path` so we can push to it.
    fn init_bare(path: &Path) {
        std::process::Command::new("git")
            .arg("init")
            .arg("--bare")
            .arg(path)
            .output()
            .unwrap();
    }

    /// Create a working repo with one initial commit and origin pointing at `bare`.
    fn make_working_clone(bare: &Path, dest: &Path, branch: &str) -> PathBuf {
        std::process::Command::new("git")
            .arg("clone")
            .arg(bare)
            .arg(dest)
            .output()
            .unwrap();
        let readme = dest.join("README.md");
        std::fs::write(&readme, b"hub\n").unwrap();
        let g = GitShellClient;
        g.add_all(dest).unwrap();
        g.commit(dest, "init", "Test", "test@example.com").unwrap();
        // The `init` clone may have created an empty default branch; force-rename to `branch`.
        std::process::Command::new("git")
            .arg("-C")
            .arg(dest)
            .arg("checkout")
            .arg("-B")
            .arg(branch)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .arg("-C")
            .arg(dest)
            .arg("push")
            .arg("-u")
            .arg("origin")
            .arg(branch)
            .output()
            .unwrap();
        dest.to_path_buf()
    }

    #[test]
    fn shell_client_round_trip() {
        let tmp = assert_fs::TempDir::new().unwrap();
        let bare = tmp.child("bare.git");
        init_bare(bare.path());

        let work = tmp.child("work");
        std::fs::create_dir_all(work.path()).unwrap();
        make_working_clone(bare.path(), work.path(), "main");

        // Now make a feature branch on top of the working clone, commit a change, push it.
        let g = GitShellClient;
        g.checkout_new_branch(work.path(), "feature").unwrap();
        std::fs::write(work.path().join("hello.txt"), b"hi\n").unwrap();
        g.add_all(work.path()).unwrap();
        let did_commit = g
            .commit(work.path(), "add hello", "Test", "t@example.com")
            .unwrap();
        assert!(did_commit);

        let url = g.push(work.path(), "origin", "feature").unwrap();
        assert!(url.contains("bare.git"));

        // Verify the branch landed in the bare repo.
        let lsremote = std::process::Command::new("git")
            .arg("ls-remote")
            .arg(bare.path())
            .arg("feature")
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&lsremote.stdout);
        assert!(s.contains("refs/heads/feature"), "branch not pushed: {}", s);
    }

    #[test]
    fn commit_returns_false_when_nothing_staged() {
        let tmp = assert_fs::TempDir::new().unwrap();
        let bare = tmp.child("bare.git");
        init_bare(bare.path());
        let work = tmp.child("work");
        std::fs::create_dir_all(work.path()).unwrap();
        make_working_clone(bare.path(), work.path(), "main");

        let g = GitShellClient;
        g.checkout_new_branch(work.path(), "feature").unwrap();
        // No add, no changes — commit should report "nothing to commit".
        let did = g.commit(work.path(), "noop", "T", "t@e").unwrap();
        assert!(!did);
    }
}
