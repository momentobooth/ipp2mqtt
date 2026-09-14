use clap::{Parser, Subcommand};

/// ipp2mqtt - A print server and status bridge that talks IPP Everywhere to printers and MQTT to everything else.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Browses for printers and prints what is found until interrupted.
    Discover,
}
