//! Per-client MCP registration snippets for `quay mcp install <client>`.
//! v1 prints the snippet + where to put it; it does not edit client files.

use std::str::FromStr;

/// MCP clients quay knows how to register with.
#[derive(Clone, Copy, Debug)]
pub enum Client {
    Claude,
    Codex,
    Cursor,
    Vscode,
    Devin,
    Opencode,
    /// Catch-all: the universal stdio shape any MCP client can adapt.
    Generic,
}

impl FromStr for Client {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Ok(Client::Claude),
            "codex" => Ok(Client::Codex),
            "cursor" => Ok(Client::Cursor),
            "vscode" | "vs-code" | "code" => Ok(Client::Vscode),
            "devin" => Ok(Client::Devin),
            "opencode" => Ok(Client::Opencode),
            "generic" | "default" | "manual" | "other" | "stdio" => Ok(Client::Generic),
            other => Err(format!(
                "unknown client '{other}' (expected: claude, codex, cursor, vscode, \
                 devin, opencode, or generic for any other MCP client)"
            )),
        }
    }
}

/// Return a ready-to-apply registration snippet plus a one-line instruction.
pub fn install_client(client: Client) -> anyhow::Result<String> {
    let out = match client {
        Client::Claude => "\
# Claude Code — run this once (user scope = available in every project):
claude mcp add -s user quay -- quay mcp"
            .to_string(),
        Client::Codex => "\
# Codex — add to ~/.codex/config.toml:
[mcp_servers.quay]
command = \"quay\"
args = [\"mcp\"]"
            .to_string(),
        Client::Cursor => "\
# Cursor — paste into .cursor/mcp.json (create the file if absent; if it
# already exists, merge the \"quay\" entry into its \"mcpServers\" object):
{
  \"mcpServers\": {
    \"quay\": {
      \"command\": \"quay\",
      \"args\": [\"mcp\"]
    }
  }
}"
        .to_string(),
        Client::Vscode => "\
# VS Code — add to .vscode/mcp.json (workspace) or your user mcp.json.
# Note: VS Code uses the \"servers\" key and requires \"type\".
# Or run once:  code --add-mcp '{\"name\":\"quay\",\"command\":\"quay\",\"args\":[\"mcp\"]}'
{
  \"servers\": {
    \"quay\": {
      \"type\": \"stdio\",
      \"command\": \"quay\",
      \"args\": [\"mcp\"]
    }
  }
}"
        .to_string(),
        Client::Devin => "\
# Devin (Devin for Terminal) — add to .devin/config.json:
# (Cloud Devin runs in its own VM — the `quay` binary must be installed
#  there too. Verify the schema at https://cli.devin.ai/docs if it changes.)
{
  \"mcpServers\": {
    \"quay\": {
      \"command\": \"quay\",
      \"args\": [\"mcp\"]
    }
  }
}"
        .to_string(),
        Client::Opencode => "\
# opencode — add to opencode.json (project) or ~/.config/opencode/opencode.json.
# Note: opencode nests under \"mcp\", type \"local\", and command is an ARRAY.
{
  \"mcp\": {
    \"quay\": {
      \"type\": \"local\",
      \"command\": [\"quay\", \"mcp\"],
      \"enabled\": true
    }
  }
}"
        .to_string(),
        Client::Generic => "\
# Any MCP client — quay speaks MCP over stdio. The facts every client needs:
#   name: quay   command: quay   args: [\"mcp\"]   transport: stdio
# Most clients (Windsurf, Zed, Claude Desktop, Gemini CLI, …) take this shape:
{
  \"mcpServers\": {
    \"quay\": {
      \"command\": \"quay\",
      \"args\": [\"mcp\"]
    }
  }
}
# Drop it in that client's MCP config file. Check its docs for the exact wrapper
# key (some use \"servers\" or \"mcp\") and file path."
            .to_string(),
    };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_clients() {
        assert!("claude".parse::<Client>().is_ok());
        assert!("claude-code".parse::<Client>().is_ok());
        assert!("CURSOR".parse::<Client>().is_ok());
        assert!("codex".parse::<Client>().is_ok());
        assert!("cursor".parse::<Client>().is_ok());
        assert!("vscode".parse::<Client>().is_ok());
        assert!("vs-code".parse::<Client>().is_ok());
        assert!("code".parse::<Client>().is_ok());
        assert!("devin".parse::<Client>().is_ok());
        assert!("opencode".parse::<Client>().is_ok());
        assert!("generic".parse::<Client>().is_ok());
        assert!("default".parse::<Client>().is_ok());
        assert!("stdio".parse::<Client>().is_ok());
        assert!("bogus".parse::<Client>().is_err());
    }

    #[test]
    fn opencode_snippet_uses_mcp_key_and_array_command() {
        let s = install_client(Client::Opencode).unwrap();
        assert!(s.contains("opencode.json"));
        // opencode nests under "mcp", type "local", command is an array.
        assert!(s.contains("\"mcp\""));
        assert!(s.contains("\"type\": \"local\""));
        assert!(s.contains("[\"quay\", \"mcp\"]"));
    }

    #[test]
    fn generic_snippet_documents_stdio_shape() {
        let s = install_client(Client::Generic).unwrap();
        assert!(s.contains("stdio"));
        assert!(s.contains("\"command\": \"quay\""));
        assert!(s.contains("\"mcp\""));
    }

    #[test]
    fn vscode_snippet_uses_servers_key_and_stdio_type() {
        let s = install_client(Client::Vscode).unwrap();
        assert!(s.contains(".vscode/mcp.json"));
        // VS Code uses "servers" (not "mcpServers") and requires a type.
        assert!(s.contains("\"servers\""));
        assert!(s.contains("\"type\": \"stdio\""));
        assert!(s.contains("\"mcp\""));
    }

    #[test]
    fn devin_snippet_targets_config_json() {
        let s = install_client(Client::Devin).unwrap();
        assert!(s.contains(".devin/config.json"));
        assert!(s.contains("\"mcpServers\""));
        assert!(s.contains("\"command\": \"quay\""));
        assert!(s.contains("\"mcp\""));
    }

    #[test]
    fn codex_snippet_is_toml_with_quay_mcp_command() {
        let s = install_client(Client::Codex).unwrap();
        assert!(s.contains("[mcp_servers.quay]"));
        assert!(s.contains("command = \"quay\""));
        assert!(s.contains("\"mcp\""));
    }

    #[test]
    fn cursor_snippet_is_json_with_quay_mcp_command() {
        let s = install_client(Client::Cursor).unwrap();
        assert!(s.contains("\"quay\""));
        assert!(s.contains("\"mcp\""));
        assert!(s.contains(".cursor/mcp.json"));
    }

    #[test]
    fn claude_snippet_shows_cli_registration() {
        let s = install_client(Client::Claude).unwrap();
        assert!(s.contains("claude mcp add"));
        // Registers at user scope so it's available in every project.
        assert!(s.contains("-s user"));
        assert!(s.contains("quay mcp"));
    }
}
