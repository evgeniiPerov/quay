#!/usr/bin/env bash
# Regenerate quay's agent registry from upstream vercel-labs/skills.
#
# Fetches src/agents.ts, runs the codegen, writes the compiled table to
# quay-core/data/agents.toml. Node runs ONLY here (CI / maintainer machine) —
# never at quay runtime. Upstream adds an agent -> re-run -> git diff -> PR.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
OUT="$REPO_ROOT/apps/cli/crates/quay-core/data/agents.toml"
UPSTREAM="${UPSTREAM_REPO:-vercel-labs/skills}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "fetching $UPSTREAM src/agents.ts ..."
gh api "repos/$UPSTREAM/contents/src/agents.ts" --jq '.content' | base64 -d > "$TMP/agents.ts"

echo "generating $OUT ..."
node "$HERE/codegen-agents.mjs" "$TMP/agents.ts" "$OUT"

echo "done. review with: git diff -- $OUT"
