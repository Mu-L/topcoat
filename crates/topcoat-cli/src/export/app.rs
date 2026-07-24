use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
    routing::get,
};
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

/// How long the already-built application gets to bind its listener and report
/// in.
const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// The local server the exported application reports its address to.
///
/// An export drives the application through the same development tooling
/// `topcoat dev` uses: the application is handed this server's URL in
/// `TOPCOAT_DEV_URL`, which both enables the development-only static route
/// listing the export reads and tells the application where to announce itself
/// once it is listening.
pub struct ReadyServer {
    /// The HTTP base URL handed to the application.
    url: String,
    /// Receives the address reported by the application.
    ready: mpsc::Receiver<Option<SocketAddr>>,
}

impl ReadyServer {
    /// Bind the server on an ephemeral local port and start serving.
    ///
    /// # Errors
    ///
    /// Returns an error if the local port cannot be bound.
    pub async fn start() -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let url = format!("http://{}", listener.local_addr()?);
        let (tx, ready) = mpsc::channel(1);

        tokio::spawn(async move {
            let app = Router::new().route("/ws", get(ws_handler)).with_state(tx);
            let _ = axum::serve(listener, app).await;
        });

        Ok(Self { url, ready })
    }

    /// The URL to hand the application in `TOPCOAT_DEV_URL`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Wait for the application to report the address it is listening on.
    ///
    /// # Errors
    ///
    /// Returns an error if the application exits first, does not report in
    /// before the timeout, or serves on a listener without a TCP address (a
    /// Unix socket, say), which an export cannot fetch pages over.
    pub async fn wait(&mut self, app: &mut AppProcess) -> Result<SocketAddr, ReadyError> {
        tokio::select! {
            reported = self.ready.recv() => match reported {
                Some(Some(addr)) => Ok(addr),
                Some(None) => Err(ReadyError::NoAddress),
                None => Err(ReadyError::Timeout),
            },
            status = app.exited() => Err(ReadyError::Exited {
                status: status.map_or_else(|error| error.to_string(), |status| status.to_string()),
            }),
            () = tokio::time::sleep(READY_TIMEOUT) => Err(ReadyError::Timeout),
        }
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(ready): State<mpsc::Sender<Option<SocketAddr>>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, ready))
}

async fn handle_socket(ws: WebSocket, ready: mpsc::Sender<Option<SocketAddr>>) {
    let (_sink, mut stream) = ws.split();
    while let Some(Ok(message)) = stream.next().await {
        if let Message::Text(text) = message
            && let Some(reported) = parse_ready(&text)
        {
            let _ = ready.send(reported.addr()).await;
            return;
        }
    }
}

/// The readiness message the framework sends once it is serving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ready {
    /// The application is listening on a TCP address.
    At(SocketAddr),
    /// The application is serving, but not over TCP (a Unix socket, say).
    Elsewhere,
}

impl Ready {
    /// The reported address, if there was one.
    fn addr(self) -> Option<SocketAddr> {
        match self {
            Self::At(addr) => Some(addr),
            Self::Elsewhere => None,
        }
    }
}

/// Parses the readiness message: `ready` on its own, or `ready <addr>` when the
/// application listens on a TCP address.
fn parse_ready(text: &str) -> Option<Ready> {
    if text == "ready" {
        return Some(Ready::Elsewhere);
    }
    text.strip_prefix("ready ")
        .map(|addr| addr.parse().map_or(Ready::Elsewhere, Ready::At))
}

/// The reason an application never became ready to export from.
#[derive(Debug)]
pub enum ReadyError {
    /// The application exited before reporting in.
    Exited {
        /// How the process ended.
        status: String,
    },
    /// The application reported in without a TCP address.
    NoAddress,
    /// The application did not report in before the timeout.
    Timeout,
}

impl std::fmt::Display for ReadyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exited { status } => {
                write!(
                    f,
                    "the application exited before serving any pages ({status})"
                )
            }
            Self::NoAddress => f.write_str(
                "the application is not listening on a TCP address, so its pages cannot be fetched",
            ),
            Self::Timeout => write!(
                f,
                "the application did not start serving within {}s; \
                 it must serve with `topcoat::serve` or `topcoat::start` to be exported",
                READY_TIMEOUT.as_secs()
            ),
        }
    }
}

impl std::error::Error for ReadyError {}

/// The application process an export renders its pages from.
pub struct AppProcess {
    child: Child,
}

impl AppProcess {
    /// Run the built executable, pointing it at `dev_url` and letting the
    /// operating system pick its port.
    ///
    /// # Errors
    ///
    /// Returns an error if the executable cannot be spawned.
    pub fn spawn(exe: &Path, dev_url: &str) -> io::Result<Self> {
        let child = Command::new(exe)
            .env("TOPCOAT_DEV_URL", dev_url)
            .env("HOST", "127.0.0.1")
            // Port 0 leaves the choice to the OS; the address comes back over
            // the readiness notification, so nothing has to be guessed.
            .env("PORT", "0")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .stdin(Stdio::null())
            // A safety net for abnormal exits; the regular path stops the
            // process explicitly via `shutdown`.
            .kill_on_drop(true)
            .spawn()?;
        Ok(Self { child })
    }

    /// Wait for the application to exit on its own.
    pub async fn exited(&mut self) -> io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }

    /// Kill the application and wait for the process to be reaped.
    pub async fn shutdown(mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_ready_message_reports_no_address() {
        assert_eq!(parse_ready("ready"), Some(Ready::Elsewhere));
    }

    #[test]
    fn a_ready_message_carries_the_listener_address() {
        assert_eq!(
            parse_ready("ready 127.0.0.1:8080"),
            Some(Ready::At("127.0.0.1:8080".parse().unwrap()))
        );
    }

    #[test]
    fn an_unparsable_address_reports_no_address() {
        assert_eq!(parse_ready("ready not-an-address"), Some(Ready::Elsewhere));
    }

    #[test]
    fn other_messages_are_ignored() {
        assert_eq!(parse_ready("reload"), None);
        assert_eq!(parse_ready(""), None);
    }
}
