mod cli;
mod commands;
mod config;
mod discovery;
mod ipp;

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
        Some(Commands::Status { filter, all_jobs }) => commands::status::run(&settings, filter.as_deref(), all_jobs).await,
        Some(Commands::Print { paths, printer, format }) => commands::print::run(&settings, &paths, printer.as_deref(), format.as_deref()).await,
        Some(Commands::Identify { filter }) => commands::control::identify(&settings, filter.as_deref()).await,
        Some(Commands::Cancel { job_id, filter }) => commands::control::cancel(&settings, job_id, filter.as_deref()).await,
        None => commands::service::run(&settings).await,
    }
}
