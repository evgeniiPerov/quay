//! Pure decision function for bulk-add collision resolution.
//!
//! `build_plan` maps a list of picked skill names + the current local skills
//! to a per-skill [`SkillAction`], driven by the caller's chosen
//! [`CollisionStrategy`].  No I/O, no `dialoguer` — fully unit-testable.

use crate::scanner::LocalSkill;

/// Per-skill action resolved by [`build_plan`] / [`build_plan_with_prompt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillAction {
    /// Skill does not exist locally — pull fresh from hub.
    Install,
    /// Skill exists locally — overwrite from hub (`add --force`).
    UpdateForce,
    /// Skill exists locally — leave it untouched.
    Skip,
}

/// Batch collision resolution strategy chosen by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionStrategy {
    /// Force-overwrite every colliding skill from hub.
    UpdateAll,
    /// Skip every colliding skill; only install new ones.
    SkipAll,
    /// Ask the user per collision (via callback or UI prompt).
    PromptEach,
}

/// Compute per-skill actions for `UpdateAll` or `SkipAll` strategies.
///
/// `picks` is the ordered list of skill names selected by the user.
/// `local` is the list of skills already present on disk.
/// Returns a `(name, action)` pair for every pick, preserving order.
pub fn build_plan(
    picks: &[&str],
    local: &[LocalSkill],
    strategy: CollisionStrategy,
) -> Vec<(String, SkillAction)> {
    assert!(
        !matches!(strategy, CollisionStrategy::PromptEach),
        "use build_plan_with_prompt for PromptEach"
    );
    let local_names: std::collections::HashSet<&str> =
        local.iter().map(|l| l.meta.name.as_str()).collect();
    picks
        .iter()
        .map(|&name| {
            let action = if local_names.contains(name) {
                match strategy {
                    CollisionStrategy::UpdateAll => SkillAction::UpdateForce,
                    CollisionStrategy::SkipAll => SkillAction::Skip,
                    CollisionStrategy::PromptEach => unreachable!("guarded above"),
                }
            } else {
                SkillAction::Install
            };
            (name.to_string(), action)
        })
        .collect()
}

/// Compute per-skill actions for the `PromptEach` strategy.
///
/// `picks` is the ordered list of skill names selected by the user.
/// `local` is the list of skills already present on disk.
/// `prompt_fn` is called for each collision with `(name, is_modified)` and
/// must return [`SkillAction::UpdateForce`] or [`SkillAction::Skip`].
///
/// `is_modified` is `true` when the local skill has [`ScanStatus::InstalledModified`].
pub fn build_plan_with_prompt<F>(
    picks: &[&str],
    local: &[LocalSkill],
    mut prompt_fn: F,
) -> Vec<(String, SkillAction)>
where
    F: FnMut(&str, bool) -> SkillAction,
{
    use crate::scanner::ScanStatus;

    let local_map: std::collections::HashMap<&str, &LocalSkill> =
        local.iter().map(|l| (l.meta.name.as_str(), l)).collect();

    picks
        .iter()
        .map(|&name| {
            let action = if let Some(local_skill) = local_map.get(name) {
                let is_modified =
                    matches!(local_skill.status, ScanStatus::InstalledModified { .. });
                prompt_fn(name, is_modified)
            } else {
                SkillAction::Install
            };
            (name.to_string(), action)
        })
        .collect()
}

/// Return the names from `picks` that are already present in `local`.
pub fn collision_names<'a>(picks: &[&'a str], local: &[LocalSkill]) -> Vec<&'a str> {
    let local_names: std::collections::HashSet<&str> =
        local.iter().map(|l| l.meta.name.as_str()).collect();
    picks
        .iter()
        .copied()
        .filter(|&n| local_names.contains(n))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{LocalLocation, ScanStatus, SkillFormat, SkillMeta};
    use std::path::PathBuf;

    fn make_local(name: &str) -> LocalSkill {
        LocalSkill {
            meta: SkillMeta {
                name: name.to_string(),
                description: String::new(),
                version: "1.0".to_string(),
                tags: vec![],
                format: SkillFormat::Frontmatter,
            },
            locations: vec![LocalLocation {
                root: crate::config::MirrorRoot::Agents,
                path: PathBuf::from(format!("/tmp/skills/{}/SKILL.md", name)),
                sha256: String::new(),
            }],
            status: ScanStatus::Local,
        }
    }

    fn make_modified(name: &str) -> LocalSkill {
        let mut s = make_local(name);
        s.status = ScanStatus::InstalledModified {
            remote: "hub".into(),
            version: "1.0".into(),
        };
        s
    }

    #[test]
    fn update_all_marks_collisions_as_force() {
        let picks = ["a", "b", "c"];
        let local = vec![make_local("b")];
        let plan = build_plan(&picks, &local, CollisionStrategy::UpdateAll);
        assert_eq!(
            plan,
            vec![
                ("a".into(), SkillAction::Install),
                ("b".into(), SkillAction::UpdateForce),
                ("c".into(), SkillAction::Install),
            ]
        );
    }

    #[test]
    fn skip_all_marks_collisions_as_skip() {
        let picks = ["a", "b", "c"];
        let local = vec![make_local("b"), make_local("a")];
        let plan = build_plan(&picks, &local, CollisionStrategy::SkipAll);
        assert_eq!(
            plan,
            vec![
                ("a".into(), SkillAction::Skip),
                ("b".into(), SkillAction::Skip),
                ("c".into(), SkillAction::Install),
            ]
        );
    }

    #[test]
    fn prompt_each_consults_callback() {
        let picks = ["a", "b", "c"];
        let local = vec![make_local("b"), make_local("c")];
        // callback: always skip
        let plan = build_plan_with_prompt(&picks, &local, |_name, _is_mod| SkillAction::Skip);
        assert_eq!(
            plan,
            vec![
                ("a".into(), SkillAction::Install),
                ("b".into(), SkillAction::Skip),
                ("c".into(), SkillAction::Skip),
            ]
        );
    }

    #[test]
    fn prompt_each_receives_is_modified_flag() {
        let picks = ["x", "y"];
        let local = vec![make_modified("x"), make_local("y")];
        let mut modified_flags: Vec<(String, bool)> = Vec::new();
        build_plan_with_prompt(&picks, &local, |name, is_mod| {
            modified_flags.push((name.to_string(), is_mod));
            SkillAction::UpdateForce
        });
        assert_eq!(
            modified_flags,
            vec![("x".into(), true), ("y".into(), false)]
        );
    }

    #[test]
    fn no_collisions_skips_strategy() {
        let picks = ["x", "y"];
        let local: Vec<LocalSkill> = vec![];
        let plan = build_plan(&picks, &local, CollisionStrategy::UpdateAll);
        assert_eq!(
            plan,
            vec![
                ("x".into(), SkillAction::Install),
                ("y".into(), SkillAction::Install),
            ]
        );
    }

    #[test]
    fn collision_names_returns_only_colliding() {
        let picks = ["a", "b", "c"];
        let local = vec![make_local("b"), make_local("d")];
        let cols = collision_names(&picks, &local);
        assert_eq!(cols, vec!["b"]);
    }
}
