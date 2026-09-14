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

    /// Probes printers over IPP and prints their capabilities and status.
    Status {
        /// Only probe printers whose name contains this text.
        filter: Option<String>,

        /// Also list completed jobs, not just active ones.
        #[arg(long)]
        all_jobs: bool,
    },

    /// Sends a document to a printer.
    Print {
        /// Files to print. More than one uses Create-Job and Send-Document.
        #[arg(required = true, num_args = 1..)]
        paths: Vec<String>,

        /// Only consider printers whose name contains this text.
        #[arg(long)]
        printer: Option<String>,

        /// Document format, for example application/pdf. Omit to let the printer detect it.
        #[arg(long)]
        format: Option<String>,
    },

    /// Asks a printer to announce itself with a sound or its display.
    Identify {
        /// Only consider printers whose name contains this text.
        filter: Option<String>,
    },

    /// Cancels a job on a printer.
    Cancel {
        /// Job id to cancel.
        job_id: i32,

        /// Only consider printers whose name contains this text.
        filter: Option<String>,
    },
}
