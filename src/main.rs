mod cli;
mod commands;
mod config;
mod discovery;

use clap::Parser;
use log::error;

use crate::cli::{Cli, Commands};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let settings = match config::loading::load_config() {
        Ok(settings) => settings,
        Err(error) => {
            error!("{error}");
            std::process::exit(1);
        }
    };

    match Cli::parse().command {
        Some(Commands::Discover) => commands::discover::run(&settings).await,
        None => commands::service::run(&settings).await,
    }
}
