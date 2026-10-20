mod cli;
mod commands;
mod config;
mod confirm;
mod gitenv;
mod http;
mod import;
mod provider;
mod rules;
mod shell;
mod store;
mod util;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    if let Err(e) = commands::dispatch(cli) {
        eprintln!("git-vault: {e:#}");
        std::process::exit(1);
    }
}
