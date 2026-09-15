use std::{collections::BTreeMap, sync::{Arc, Mutex}, time::Duration};

use exponential_backoff::Backoff;
use log::{debug, error, info, warn};
use rumqttc::{AsyncClient, ClientError, Event, EventLoop, LastWill, MqttOptions, Packet, QoS, Transport};
use snafu::{ResultExt, Snafu};
use tokio::sync::mpsc;

use crate::config::models::Mqtt;

use super::tls::{self, TlsError};

/// Request channel size. rumqttc's `set_request_channel_capacity` is not wired
/// to anything; only this value reaches the channel.
const REQUEST_CAPACITY: usize = 256;

const INBOUND_CAPACITY: usize = 64;

/// Bounds on how long to wait before reconnecting. rumqttc retries immediately
/// and has no backoff of its own, so without this a refused connection turns
/// into a reconnect storm.
const RECONNECT_MIN: Duration = Duration::from_secs(1);
const RECONNECT_MAX: Duration = Duration::from_secs(60);

pub const ONLINE: &[u8] = b"online";
pub const OFFLINE: &[u8] = b"offline";

/// Something that arrived from the broker.
#[derive(Debug, Clone)]
pub enum MqttEvent {
    /// The broker accepted a connection. Emitted for the first connection and
    /// every reconnection, so consumers can republish state they own.
    Connected,
    Disconnected,
    Message { topic: String, payload: Vec<u8> },
}

/// Publishes to and subscribes on an MQTT broker, reconnecting as needed.
///
/// Cheap to clone; every clone shares one connection.
#[derive(Clone)]
pub struct MqttClient {
    client: AsyncClient,
    /// Every subscription this client wants. The broker forgets subscriptions on
    /// reconnect and rumqttc does not replay them, so they are replayed here.
    subscriptions: Arc<Mutex<BTreeMap<String, u8>>>,
    /// Last payload published per topic, so unchanged state is not resent.
    published: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    availability_topic: String,
    qos: QoS,
}

impl MqttClient {
    /// Connects and starts the event loop. Returns the client and the stream of
    /// inbound events.
    pub fn connect(settings: &Mqtt) -> Result<(Self, mpsc::Receiver<MqttEvent>), MqttError> {
        let availability_topic = format!("{}/bridge", settings.root_topic);
        let qos = qos_from(settings.qos)?;

        let mut options = MqttOptions::new(settings.client_id.clone(), settings.host.clone(), settings.port);
        options
            .set_keep_alive(settings.keep_alive)
            .set_clean_session(settings.clean_session)
            .set_max_packet_size(settings.max_packet_size, settings.max_packet_size)
            .set_last_will(LastWill::new(&availability_topic, OFFLINE, qos, true));

        if !settings.username.is_empty() {
            options.set_credentials(settings.username.clone(), settings.password.expose().to_owned());
        }

        if settings.secure {
            let config = tls::client_config(settings).context(TlsSnafu)?;
            options.set_transport(Transport::tls_with_config(config.into()));
        }

        let (client, event_loop) = AsyncClient::new(options, REQUEST_CAPACITY);
        let (sender, receiver) = mpsc::channel(INBOUND_CAPACITY);

        let mqtt = MqttClient {
            client,
            subscriptions: Arc::new(Mutex::new(BTreeMap::new())),
            published: Arc::new(Mutex::new(BTreeMap::new())),
            availability_topic,
            qos,
        };

        tokio::spawn(run(mqtt.clone(), event_loop, sender));
        Ok((mqtt, receiver))
    }

    pub fn availability_topic(&self) -> &str {
        &self.availability_topic
    }

    /// Queues a publish.
    ///
    /// Never awaits: rumqttc's `publish` parks once its request channel fills,
    /// and while the broker is unreachable nothing drains that channel, so an
    /// awaiting caller would hang for as long as the outage lasts. A status
    /// bridge would rather drop a reading than wedge the tasks producing them.
    pub fn publish(&self, topic: &str, payload: &[u8], retain: bool) -> Result<(), MqttError> {
        if retain {
            self.published.lock().unwrap().insert(topic.to_owned(), payload.to_vec());
        }
        self.client.try_publish(topic, self.qos, retain, payload).context(PublishSnafu { topic: topic.to_owned() })
    }

    /// Publishes only when the payload differs from what was last published to
    /// this topic, which keeps a poll loop from resending unchanged state.
    pub fn publish_changed(&self, topic: &str, payload: &[u8]) -> Result<(), MqttError> {
        if self.published.lock().unwrap().get(topic).is_some_and(|last| last == payload) {
            return Ok(());
        }
        self.publish(topic, payload, true)
    }

    /// Removes a retained topic.
    ///
    /// A zero length retained payload deletes the stored message rather than
    /// replacing it with an empty one.
    pub fn clear_retained(&self, topic: &str) -> Result<(), MqttError> {
        self.published.lock().unwrap().remove(topic);
        self.client.try_publish(topic, self.qos, true, Vec::new()).context(PublishSnafu { topic: topic.to_owned() })
    }

    /// Records a subscription and applies it. Recorded subscriptions are
    /// replayed on every reconnection.
    pub fn subscribe(&self, filter: &str) -> Result<(), MqttError> {
        self.subscriptions.lock().unwrap().insert(filter.to_owned(), self.qos as u8);
        self.client.try_subscribe(filter, self.qos).context(SubscribeSnafu { filter: filter.to_owned() })
    }

    /// Announces the bridge as offline and disconnects.
    ///
    /// The offline message is published explicitly because a clean DISCONNECT
    /// tells the broker to discard the will, which would otherwise leave the
    /// availability topic reading `online` forever.
    pub async fn shutdown(&self) {
        if let Err(error) = self.publish(&self.availability_topic.clone(), OFFLINE, true) {
            warn!("Could not publish offline state: {error}");
        }
        if let Err(error) = self.client.disconnect().await {
            debug!("Could not disconnect cleanly: {error}");
        }
    }

    /// Re-applies every recorded subscription. Called on each connection.
    fn resubscribe(&self) {
        let subscriptions = self.subscriptions.lock().unwrap().clone();
        for (filter, qos) in subscriptions {
            let qos = qos_from(qos).unwrap_or(self.qos);
            // try_subscribe, never subscribe: this runs on the event loop task,
            // which is the only thing that drains the request channel, so an
            // awaiting call here would deadlock the connection permanently.
            if let Err(error) = self.client.try_subscribe(filter.clone(), qos) {
                warn!("Could not subscribe to {filter}: {error}");
            }
        }
    }
}

/// Drives the connection. rumqttc reconnects by itself as long as polling
/// continues, so the loop never rebuilds the event loop; it only paces retries.
async fn run(mqtt: MqttClient, mut event_loop: EventLoop, events: mpsc::Sender<MqttEvent>) {
    let backoff = Backoff::new(u32::MAX, RECONNECT_MIN, RECONNECT_MAX);
    let mut delays = backoff.iter();
    let mut connected = false;

    loop {
        match event_loop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                info!("Connected to MQTT broker (session present: {})", ack.session_present);
                delays = backoff.iter();
                connected = true;

                // Both of these belong here rather than at startup: this arm runs
                // for the first connection and every reconnection alike.
                if let Err(error) = mqtt.publish(&mqtt.availability_topic.clone(), ONLINE, true) {
                    warn!("Could not publish online state: {error}");
                }
                mqtt.resubscribe();

                if events.send(MqttEvent::Connected).await.is_err() {
                    break;
                }
            }
            Ok(Event::Incoming(Packet::Publish(publish))) => {
                let message = MqttEvent::Message {
                    topic: publish.topic,
                    payload: publish.payload.to_vec(),
                };
                if events.send(message).await.is_err() {
                    break;
                }
            }
            Ok(Event::Outgoing(rumqttc::Outgoing::Disconnect)) => {
                debug!("Disconnect sent, stopping MQTT event loop");
                break;
            }
            Ok(_) => {}
            Err(error) => {
                if connected {
                    warn!("MQTT connection lost: {error}");
                    connected = false;
                    if events.send(MqttEvent::Disconnected).await.is_err() {
                        break;
                    }
                } else {
                    debug!("MQTT connection attempt failed: {error}");
                }

                match delays.next().flatten() {
                    Some(delay) => tokio::time::sleep(delay).await,
                    None => tokio::time::sleep(RECONNECT_MAX).await,
                }
            }
        }
    }

    error!("MQTT event loop stopped");
}

fn qos_from(qos: u8) -> Result<QoS, MqttError> {
    match qos {
        0 => Ok(QoS::AtMostOnce),
        1 => Ok(QoS::AtLeastOnce),
        2 => Ok(QoS::ExactlyOnce),
        other => Err(MqttError::Qos { qos: other }),
    }
}

// ////// //
// Errors //
// ////// //

#[derive(Debug, Snafu)]
pub enum MqttError {
    #[snafu(display("Could not configure MQTT TLS: {source}"))]
    Tls { source: TlsError },

    #[snafu(display("Could not publish to {topic}: {source}"))]
    Publish { topic: String, source: ClientError },

    #[snafu(display("Could not subscribe to {filter}: {source}"))]
    Subscribe { filter: String, source: ClientError },

    #[snafu(display("{qos} is not a valid QoS; use 0, 1 or 2"))]
    Qos { qos: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_three_defined_qos_levels_are_accepted() {
        assert!(matches!(qos_from(0), Ok(QoS::AtMostOnce)));
        assert!(matches!(qos_from(1), Ok(QoS::AtLeastOnce)));
        assert!(matches!(qos_from(2), Ok(QoS::ExactlyOnce)));
        assert!(matches!(qos_from(3), Err(MqttError::Qos { qos: 3 })));
    }
}
