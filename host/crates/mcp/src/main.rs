//! `perritodeck-mcp`: the stdio bridge AI assistants start (see `perritodeck_mcp::cli`).

fn main() {
    std::process::exit(perritodeck_mcp::cli::run(std::env::args_os().skip(1)));
}
