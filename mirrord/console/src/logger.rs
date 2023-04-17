use std::{
    io::{Read, Write},
    sync::mpsc::{sync_channel, Receiver, SyncSender},
    thread,
};

use log::{LevelFilter, Metadata};
use tungstenite::{connect, protocol::Message, WebSocket};

use crate::{
    error::{ConsoleError, Result},
    protocol,
};

/// Console logger that sends log messages to the console app.
pub struct ConsoleLogger {
    sender: SyncSender<protocol::Record>,
}

impl log::Log for ConsoleLogger {
    /// Returns true if the log is generated by mirrord code.
    /// We can have this more fine-grained and also inclusive but
    /// be aware that you might get into a recursive scenario if you let
    /// websocket module logs slide in.
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.target().contains("mirrord")
    }

    /// Serialize the logs into our protocol then send it over the wire.
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            match self.sender.send(protocol::Record {
                metadata: protocol::Metadata {
                    level: record.level(),
                    target: record.target().to_string(),
                },
                message: record.args().to_string(),
                module_path: record.module_path().map(|s| s.to_string()),
                file: record.file().map(|s| s.to_string()),
                line: record.line(),
            }) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Error sending log message: {e:?}");
                }
            }
        }
    }

    fn flush(&self) {}
}

/// Send hello message, containing information about the connected process.
fn send_hello<S: Read + Write>(client: &mut WebSocket<S>) -> Result<()> {
    let hello = protocol::Hello {
        process_info: protocol::ProcessInfo {
            args: std::env::args().collect(),
            env: std::env::vars().map(|(k, v)| format!("{k}={v}")).collect(),
            cwd: std::env::current_dir()
                .map(|p| p.to_str().map(String::from))
                .unwrap_or(None),
            id: std::process::id().into(),
        },
    };
    let msg = Message::binary(serde_json::to_vec(&hello).unwrap());
    client.write_message(msg)?;
    Ok(())
}

/// Background task that does the communication
/// with the console app.
fn logger_task<S: Read + Write>(mut client: WebSocket<S>, rx: Receiver<protocol::Record>) {
    while let Ok(msg) = rx.recv() {
        let msg = Message::binary(serde_json::to_vec(&msg).unwrap());
        if let Err(err) = client.write_message(msg) {
            eprintln!("Error sending log message: {err:?}");
            break;
        }
    }
}

/// Initializes the logger
/// Connects to the console, and sets the global logger to use it.
pub fn init_logger(address: &str) -> Result<()> {
    let (tx, rx) = sync_channel(10000);
    let (mut client, _) =
        connect(format!("ws://{address}/ws")).map_err(ConsoleError::ConnectError)?;

    send_hello(&mut client)?;
    thread::spawn(move || {
        logger_task(client, rx);
    });
    let logger = ConsoleLogger { sender: tx };
    log::set_boxed_logger(Box::new(logger)).map(|()| log::set_max_level(LevelFilter::Trace))?;
    Ok(())
}
