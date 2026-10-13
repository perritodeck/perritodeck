//! What the person pastes into an assistant's settings so it finds PerritoDeck.
//!
//! Most assistants (Claude Desktop, Cursor, Windsurf, …) read an `mcpServers` object
//! keyed by a server name; each entry either launches a command (stdio) or points at a
//! URL (Streamable HTTP).

use serde_json::{json, Value};

/// The name assistants list PerritoDeck under.
pub const SERVER_KEY: &str = "perritodeck";

/// For assistants that launch a command, e.g. the stdio bridge `perritodeck-mcp`, which
/// finds the running host by itself:
///
/// ```json
/// { "mcpServers": { "perritodeck": { "command": "/path/to/perritodeck-mcp", "args": [] } } }
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
/// `claude mcp add --scope user perritodeck "--" /path/to/perritodeck-mcp`.
///
/// The commands are meant to work pasted into bash (Git Bash is what Claude Code uses
/// on Windows), PowerShell and cmd alike:
///
/// * `--scope user` makes PerritoDeck available in every folder. Claude Code's default
///   scope is the folder the command was run in.
/// * The `--` that ends Claude Code's own options is quoted because PowerShell drops a
///   bare `--` on its way into a script, and `claude` is npm's `claude.ps1` there. The
///   bridge's own options (`--data-dir`) would then be taken for Claude Code's.
pub fn claude_code_stdio_command(command: &str, args: &[&str]) -> String {
    let mut out = format!("claude mcp add --scope user {SERVER_KEY} \"--\"");
    for part in std::iter::once(command).chain(args.iter().copied()) {
        out.push(' ');
        out.push_str(&shell_quote(part));
    }
    out
}

/// A Claude Code command line that adds the HTTP endpoint.
pub fn claude_code_http_command(url: &str, token: &str) -> String {
    format!(
        "claude mcp add --scope user --transport http {SERVER_KEY} {} --header {}",
        shell_quote(url),
        shell_quote(&format!("Authorization: Bearer {token}"))
    )
}

/// Quote for a shell when needed. Double quotes work the same in POSIX shells,
/// PowerShell and cmd for text without quotes, `$` or `` ` `` in it (though a network
/// path's leading `\\` still loses one backslash in bash).
///
/// Anything with a backslash is quoted, so Windows paths always are: outside quotes,
/// bash takes a backslash as an escape and `C:\Users\me` comes out as `C:Usersme`.
/// Backslashes just before the closing quote are doubled, or they'd escape it, both in
/// bash and in how Windows programs split their command line (`"D:\data\"`).
fn shell_quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/:._-=@+,".contains(c));
    if plain {
        return s.to_owned();
    }
    let body = s.replace('"', "\\\"");
    let trailing = body.len() - body.trim_end_matches('\\').len();
    format!("\"{body}{}\"", "\\".repeat(trailing))
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
            r#"claude mcp add --scope user perritodeck "--" "C:\Program Files\PerritoDeck\perritodeck-host.exe" mcp"#
        );
        assert_eq!(
            claude_code_http_command("http://127.0.0.1:46711/mcp", "abc"),
            r#"claude mcp add --scope user --transport http perritodeck http://127.0.0.1:46711/mcp --header "Authorization: Bearer abc""#
        );
    }

    #[test]
    fn windows_paths_are_quoted_even_without_spaces() {
        // Where the installer puts the app, with a data folder that isn't the usual one.
        assert_eq!(
            claude_code_stdio_command(
                r"C:\Users\me\AppData\Local\PerritoDeck\perritodeck-app.exe",
                &["mcp", "--data-dir", r"D:\pd"]
            ),
            r#"claude mcp add --scope user perritodeck "--" "C:\Users\me\AppData\Local\PerritoDeck\perritodeck-app.exe" mcp --data-dir "D:\pd""#
        );
        assert_eq!(
            claude_code_stdio_command("/usr/bin/perritodeck-app", &["mcp"]),
            r#"claude mcp add --scope user perritodeck "--" /usr/bin/perritodeck-app mcp"#
        );
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("mcp"), "mcp");
        assert_eq!(shell_quote("--data-dir"), "--data-dir");
        assert_eq!(shell_quote(""), r#""""#);
        assert_eq!(
            shell_quote(r"I:\x\target\debug\a.exe"),
            r#""I:\x\target\debug\a.exe""#
        );
        // A backslash right before the closing quote would escape it.
        assert_eq!(shell_quote(r"D:\data\"), r#""D:\data\\""#);
        assert_eq!(shell_quote(r"D:\"), r#""D:\\""#);
        assert_eq!(shell_quote(r#"say "hi""#), r#""say \"hi\"""#);
    }
}
