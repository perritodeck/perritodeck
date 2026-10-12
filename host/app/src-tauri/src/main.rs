// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `perritodeck-app mcp …`: act as the stdio bridge for AI assistants, no window.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("mcp") {
        std::process::exit(perritodeck_app_lib::run_mcp_bridge(&args[1..]));
    }
    perritodeck_app_lib::run();
}
