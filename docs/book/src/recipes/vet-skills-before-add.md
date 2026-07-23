# Vet a skill before `quay add`

`quay add` fetches Markdown that your agent will execute with implicit trust.
quay verifies *provenance and integrity* — that the file you got is the file the
remote has. It does not read the skill and judge whether the instructions inside
it are hostile. Nothing in quay does.

[SkillSpector](https://github.com/NVIDIA/SkillSpector) (NVIDIA, Apache-2.0) does
that part: 68 patterns across 17 categories — prompt injection, data
exfiltration, privilege escalation, supply chain, excessive agency, MCP tool
poisoning, plus AST-level dangerous-code detection and taint tracking.

The two tools are not integrated and do not need to be. Run the scan, then add.

## Scan before you add

```sh
# Scan the source repo without downloading it yourself
skillspector scan https://github.com/someone/their-skill --no-llm

# Happy with it?
quay add their-skill
```

Exit codes: `0` = risk score ≤ 50, `1` = over threshold, `2` = scan error. The
threshold is hard-coded in SkillSpector; there is no flag to move it.

`--no-llm` runs static analysis only. Without it, skill content is sent to an
LLM provider for semantic evaluation — worth it for a manual review of something
you distrust, wrong for anything automatic.

## Audit what you already installed

```sh
# Everything in canonical, one summary table
skillspector scan .agents/skills/ --recursive --no-llm
```

Findings you have triaged and accepted go in a baseline so re-scans surface only
what is new:

```sh
skillspector baseline .agents/skills/ -o .skillspector-baseline.yaml
skillspector scan .agents/skills/ --recursive --no-llm --baseline .skillspector-baseline.yaml
```

Commit the baseline.

## Gate it in CI

quay's own repo runs [`.github/workflows/skill-security.yml`](https://github.com/evgeniiPerov/quay/blob/main/.github/workflows/skill-security.yml)
on every PR that touches `.agents/skills/`, uploading SARIF to GitHub code
scanning. Copy it. Two things it works around:

- `--recursive` with `--format sarif` writes concatenated text, **not** merged
  SARIF. Scan each skill directory separately and point `upload-sarif` at the
  output directory.
- A skill over threshold exits `1` mid-loop. Collect failures and fail the job
  at the end, or one bad skill hides the rest.

Pin the install to a commit. SkillSpector publishes no tagged releases and moves
fast; an unpinned `git+https://` install means your CI gate changes without you.

## Let the agent do it

Both tools ship MCP servers — `quay mcp` and `skillspector mcp`. An agent with
both configured can scan and then install in one flow, gating the install on the
result, with no wiring between the projects:

```jsonc
{
  "mcpServers": {
    "quay":         { "command": "quay",         "args": ["mcp"] },
    "skillspector": { "command": "skillspector", "args": ["mcp"] }
  }
}
```

Requires the MCP extra: `uv tool install 'skillspector[mcp] @ git+https://github.com/NVIDIA/skillspector.git'`.

## Why this isn't built into quay

A pre-install gate inside `quay add` would mean a Python runtime in a Rust
tool's install path, and a hard dependency on a CLI contract that has no
released version yet. Scanning stays a separate step you compose, by choice.
