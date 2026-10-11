//! What the person pastes into an assistant's settings so it finds PerritoDeck.
//!
//! Most assistants (Claude Desktop, Cursor, Windsurf, …) read an `mcpServers` object
//! keyed by a server name; each entry either launches a command (stdio) or points at a
//! URL (Streamable HTTP).

use serde_json::{json, Value};

/// The name assistants list PerritoDeck under.
pub const SERVER_KEY: &str = "perritodeck";

/// For assistants that launch a command, e.g. the host binary with `["mcp"]` (the
/// stdio bridge, which finds the running host by itself):
///
/// ```json
/// { "mcpServers": { "perritodeck": { "command": "/path/to/perritodeck-host", "args": ["mcp"] } } }
/// ```
pub fn stdio_client_config(command: &str, args: &[&str]) -> Value {
    json!({ "mcpServers": { SERVER_KEY: { "command": command, "args": args } } })
}

/// For assistants that connect to a URL:
///
/// ```json
/// { "mcpServers": { "perritodeck": { "type": "http", "url": "http://127.0.0.1:46711/mcp",
///   "headers": { "Authorization": "Bearer <token>" } } } }
/// ```
pub fn http_client_config(url: &str, token: &str) -> Value {
    json!({
        "mcpServers": {
            SERVER_KEY: {
                "type": "http",
                "url": url,
                "headers": { "Authorization": format!("Bearer {token}") },
            }
        }
    })
}

/// A Claude Code command line that adds the stdio bridge:
/// `claude mcp add perritodeck -- /path/to/perritodeck-host mcp`.
pub fn claude_code_stdio_command(command: &str, args: &[&str]) -> String {
    let mut out = format!("claude mcp add {SERVER_KEY} --");
    for part in std::iter::once(command).chain(args.iter().copied()) {
        out.push(' ');
        out.push_str(&shell_quote(part));
    }
    out
}

/// A Claude Code command line that adds the HTTP endpoint.
pub fn claude_code_http_command(url: &str, token: &str) -> String {
    format!(
        "claude mcp add --transport http {SERVER_KEY} {} --header {}",
        shell_quote(url),
        shell_quote(&format!("Authorization: Bearer {token}"))
    )
}

/// Quote for a shell when needed: double quotes work in POSIX shells, PowerShell and
/// cmd alike for paths without quotes or `$` in them.
fn shell_quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/\\:._-=@+,".contains(c));
    if plain {
        s.to_owned()
    } else {
        format!("\"{}\"", s.replace('"', "\\\""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets() {
        let c = stdio_client_config(
            r"C:\Program Files\PerritoDeck\perritodeck-host.exe",
            &["mcp"],
        );
        assert_eq!(c["mcpServers"]["perritodeck"]["args"], json!(["mcp"]));
        let h = http_client_config("http://127.0.0.1:46711/mcp", "abc");
        assert_eq!(
            h["mcpServers"]["perritodeck"]["headers"]["Authorization"],
            "Bearer abc"
        );
        assert_eq!(h["mcpServers"]["perritodeck"]["type"], "http");
        assert_eq!(
            claude_code_stdio_command(
                r"C:\Program Files\PerritoDeck\perritodeck-host.exe",
                &["mcp"]
            ),
            r#"claude mcp add perritodeck -- "C:\Program Files\PerritoDeck\perritodeck-host.exe" mcp"#
        );
        assert_eq!(
            claude_code_http_command("http://127.0.0.1:46711/mcp", "abc"),
            r#"claude mcp add --transport http perritodeck http://127.0.0.1:46711/mcp --header "Authorization: Bearer abc""#
        );
    }
}
