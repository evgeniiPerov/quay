read AGENTS.md + skim docs/superpowers/plans/, then dispatch Plan 6.75 (Create/Push + onboarding)

  or

  read AGENTS.md, dispatch Plan 7 (providers + distribution)

  or just

  what's next on quay?

  Memory files already saved at /home/evgenii/.claude/projects/-home-evgenii-projects-quay/memory/ cover: agents skip commits, caveman mode
  default, project status, repo layout. Auto-loaded.

  Quick state check next session — paste this:

  cd /home/evgenii/projects/quay && cargo test --manifest-path apps/cli/Cargo.toml 2>&1 | grep -E "^test result:" | awk '{p+=$4;i+=$8} END {print
  "tests:",p,"ignored:",i}' && git status --short | wc -l


Just open new session in /home/evgenii/projects/quay and say "what's next" or "do plan 6.75". Memory loads automatically.
