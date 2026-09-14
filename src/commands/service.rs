use log::{error, info};

use crate::{commands::discover::log_event, config::models::Settings, discovery::service::DiscoveryService};

/// Runs the bridge.
pub async fn run(settings: &Settings) {
    info!("Starting ipp2mqtt v{}", env!("CARGO_PKG_VERSION"));
    info!("Running with config: {settings:#?}");

    let mut events = match DiscoveryService::start(settings) {
        Ok(events) => events,
        Err(error) => {
            error!("{error}");
            std::process::exit(1);
        }
    };

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            event = events.recv() => match event {
                Some(event) => log_event(&event),
                None => break,
            },
        }
    }

    info!("Shutting down");
}
