//! The work behind the Speed tab: the streams of events a running test and the test server send.

use iced::futures::SinkExt;
use lanping_core::ndt7::{self, Server};
use lanping_core::ping::tcp_ping;
use lanping_core::speedtest::{self, SERVICE_HOST, SERVICE_PORT};
use lanping_core::throughput::{self, Direction, ThroughputResult, run_client};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::app::Message;
use crate::speed_model::*;

const STREAM_BUFFER: usize = 64;

/// Where a started test runs, with what it needs to get there.
pub(crate) enum Target {
    Cloudflare,
    /// A server found moments ago, whose address still works.
    MlabFresh(Server),
    /// A server to look up again first, because its address may have expired.
    Mlab {
        location: ndt7::Location,
        machine: String,
    },
    Lan {
        host: String,
        port: u16,
    },
}

/// Stops the task it holds when dropped, so a stopped stream does not leave work running.
pub(crate) struct StopOnDrop<T>(JoinHandle<T>);

impl<T> Drop for StopOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Runs a test and reports what happens, step by step.
pub(crate) fn test_stream(
    target: Target,
    kind: TestKind,
    seconds: u8,
    generation: u64,
) -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(STREAM_BUFFER, async move |mut output| {
        let (events, mut incoming) = mpsc::unbounded_channel();
        let _job = StopOnDrop(tokio::spawn(run_steps(target, kind, seconds, events)));
        while let Some(event) = incoming.recv().await {
            if output
                .send(Message::SpeedEvent(generation, event))
                .await
                .is_err()
            {
                return;
            }
        }
    })
}

/// A target ready to be measured. For M-Lab that is a server with a fresh address.
enum Prepared {
    Cloudflare,
    Mlab(Server),
    Lan { host: String, port: u16 },
}

/// Asks M-Lab for fresh server addresses, since the ones found earlier expire, and picks
/// the chosen machine, or the first one offered if it is gone.
async fn prepare(target: Target) -> Result<Prepared, String> {
    match target {
        Target::Cloudflare => Ok(Prepared::Cloudflare),
        Target::Lan { host, port } => Ok(Prepared::Lan { host, port }),
        Target::MlabFresh(server) => Ok(Prepared::Mlab(server)),
        Target::Mlab { location, machine } => {
            let servers = ndt7::locate(&location).await?;
            servers
                .iter()
                .find(|server| server.machine == machine)
                .or(servers.first())
                .cloned()
                .map(Prepared::Mlab)
                .ok_or_else(|| "M-Lab has no test server there".to_owned())
        }
    }
}

impl Prepared {
    /// The host and port a ping goes to: where the test connects.
    fn address(&self) -> Result<(String, u16), String> {
        match self {
            Self::Cloudflare => Ok((SERVICE_HOST.to_owned(), SERVICE_PORT)),
            Self::Mlab(server) => server.address(),
            Self::Lan { host, port } => Ok((host.clone(), *port)),
        }
    }

    async fn measure(
        &self,
        direction: Direction,
        seconds: u8,
        progress: mpsc::UnboundedSender<f64>,
    ) -> Result<ThroughputResult, String> {
        match self {
            Self::Cloudflare => speedtest::run(direction, seconds, Some(progress)).await,
            Self::Mlab(server) => ndt7::run(server, direction, seconds, Some(progress)).await,
            Self::Lan { host, port } => {
                run_client(host, *port, direction, seconds, Some(progress)).await
            }
        }
    }
}

pub(crate) fn mlab_place(server: &Server) -> String {
    format!("Internet (M-Lab, {})", server.place())
}

/// Ping, then each direction the kind asks for. Always ends with `Finished` or `Failed`.
async fn run_steps(
    target: Target,
    kind: TestKind,
    seconds: u8,
    events: mpsc::UnboundedSender<SpeedEvent>,
) {
    let send = |event: SpeedEvent| {
        let _ = events.send(event);
    };
    let prepared = match prepare(target).await {
        Ok(prepared) => prepared,
        Err(message) => return send(SpeedEvent::Failed(message)),
    };
    if let Prepared::Mlab(server) = &prepared {
        send(SpeedEvent::Place(mlab_place(server)));
    }
    send(SpeedEvent::Phase(Phase::Ping));
    match prepared.address() {
        Ok((host, port)) => match tcp_ping(&host, port).await {
            Ok(ping) => send(SpeedEvent::Ping(ping)),
            Err(message) => send(SpeedEvent::PingFailed(message)),
        },
        Err(message) => send(SpeedEvent::PingFailed(message)),
    }
    for &direction in kind.directions() {
        send(SpeedEvent::Phase(match direction {
            Direction::Download => Phase::Download,
            Direction::Upload => Phase::Upload,
        }));
        let (progress, mut ticks) = mpsc::unbounded_channel();
        let forwarder = tokio::spawn({
            let events = events.clone();
            async move {
                while let Some(mbps) = ticks.recv().await {
                    let _ = events.send(SpeedEvent::Progress(mbps));
                }
            }
        });
        let result = prepared.measure(direction, seconds, progress).await;
        let _ = forwarder.await;
        match result {
            Ok(result) => send(SpeedEvent::Result(result)),
            Err(message) => return send(SpeedEvent::Failed(message)),
        }
    }
    send(SpeedEvent::Finished);
}

/// Listens for tests and reports each stream it serves, until stopped.
pub(crate) fn server_stream(port: u16) -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(STREAM_BUFFER, async move |mut output| {
        let listener = match throughput::bind(port).await {
            Ok(listener) => listener,
            Err(error) => {
                let message = format!("Could not listen on port {port}: {error}");
                let _ = output.send(Message::SpeedServerFailed(message)).await;
                return;
            }
        };
        let actual_port = listener.local_addr().map_or(port, |address| address.port());
        let _ = output
            .send(Message::SpeedServerListening(
                actual_port,
                throughput::local_addresses(),
            ))
            .await;
        let (served, mut events) = mpsc::unbounded_channel();
        let _server = StopOnDrop(tokio::spawn(throughput::serve(listener, served)));
        while let Some(stream) = events.recv().await {
            if output.send(Message::SpeedServed(stream)).await.is_err() {
                return;
            }
        }
    })
}
