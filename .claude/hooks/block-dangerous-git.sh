#!/bin/bash
# PreToolUse guard for Bash: hard-blocks the git operations .agents/rules/git-policy.md
# forbids. A plain `git push` is allowed (policy permits it on explicit request);
# only history-rewriting pushes are blocked. Exit 2 = block, stderr goes to Claude.

COMMAND=$(jq -r '.tool_input.command // empty')

block() {
  echo "BLOCKED by .claude/hooks/block-dangerous-git.sh: $1. If this is really intended, the user runs it themselves with \`! <command>\`." >&2
  exit 2
}

# ponytail: text match, not a shell parser. Quoted strings are NOT stripped, so a
# `bash -c "git push -f"` can't sneak past; the cost is false positives when a
# message/body quotes a blocked command — pass those via -F / --body-file instead.
# Split on shell separators so `git status && git push -f` is checked per segment.
# `git [-C dir] [-c k=v ...] <sub>` at a word boundary (also matches `rtk git <sub>`),
# anchored on the subcommand so text inside quoted messages doesn't trip it.
G='(^|[[:space:]])git([[:space:]]+-[^[:space:]]+([[:space:]]+[^-[:space:]][^[:space:]]*)?)*[[:space:]]+'
while IFS= read -r seg; do
  if [[ $seg =~ ${G}push([[:space:]]|$)(.*) ]]; then
    args=" ${BASH_REMATCH[5]} "
    [[ $args =~ [[:space:]]--(force|force-with-lease|force-if-includes|mirror)([=[:space:]]) ]] && block "force push (--${BASH_REMATCH[1]})"
    [[ $args =~ [[:space:]]-[a-zA-Z]*f[a-zA-Z]*[[:space:]] ]] && block "force push (-f)"
    [[ $args =~ [[:space:]]\+[^[:space:]] ]] && block "force push (+refspec)"
  fi
  [[ $seg =~ ${G}reset[[:space:]]+(.*[[:space:]])?--hard([[:space:]]|$) ]] && block "git reset --hard"
  [[ $seg =~ ${G}clean[[:space:]]+(.*[[:space:]])?(-[a-zA-Z]*f[a-zA-Z]*|--force)([[:space:]]|$) ]] && block "git clean -f"
  [[ $seg =~ ${G}branch[[:space:]]+(.*[[:space:]])?(-D|--delete[[:space:]]+--force)([[:space:]]|$) ]] && block "git branch -D"
  [[ $seg =~ ${G}(checkout|restore)[[:space:]]+(--[[:space:]]+)?\.([[:space:]]|$) ]] && block "discarding all working-tree changes"
  [[ $seg =~ ${G}stash[[:space:]]+(drop|clear) ]] && block "git stash drop/clear"
  [[ $seg =~ ${G}filter-branch ]] && block "git filter-branch"
done < <(printf '%s\n' "$COMMAND" | sed -E 's/(&&|\|\||;|\|)/\n/g')

exit 0
