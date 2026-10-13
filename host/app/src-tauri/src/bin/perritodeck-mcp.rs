//! `perritodeck-mcp`, the stdio bridge for AI assistants, built and bundled with the app
//! (Tauri bundles every binary of this package). The app copies it into its data folder
//! at start, and assistants start that copy; see `perritodeck_mcp::cli`.

fn main() {
    std::process::exit(perritodeck_mcp::cli::run(std::env::args_os().skip(1)));
}
