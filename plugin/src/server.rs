//! The Unix socket the client connects to, and the state the OBS side shares with it.

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use odl_proto::{ClientMsg, ServerMsg};

use crate::encoder::Codec;
use crate::output::{self, Action};

pub struct Client {
    id: u64,
    stream: UnixStream,
}

#[derive(Clone, Copy, Debug)]
pub struct Request {
    pub codec: Codec,
}

#[derive(Default)]
pub struct Shared {
    client: Mutex<Option<Client>>,
    pub request: Mutex<Option<Request>>,
    /// Set by a client's keyframe request, cleared by the encoder thread.
    pub keyframe: AtomicBool,
    pub bitrate_kbps: AtomicU32,
    next_id: AtomicU64,
    quit: AtomicBool,
}

impl Shared {
    /// Sends to the current client. A failed write drops the client and stops
    /// the output, the same as a disconnect.
    pub fn send(&self, msg: &ServerMsg) {
        let mut guard = self.client.lock().unwrap();
        let Some(client) = guard.as_mut() else { return };
        if let Err(e) = odl_proto::write(&mut client.stream, &msg.encode()) {
            crate::warn(&format!("client {}: write: {e}; dropping it", client.id));
            let _ = client.stream.shutdown(std::net::Shutdown::Both);
            *guard = None;
            drop(guard);
            output::queue(Action::Stop);
        }
    }
}

pub struct Server {
    path: PathBuf,
    accept: Option<JoinHandle<()>>,
    readers: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

pub fn spawn(shared: Arc<Shared>) -> io::Result<Server> {
    let path = odl_proto::socket_path();
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    crate::info(&format!("listening on {}", path.display()));
    let readers: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::default();
    let r = readers.clone();
    let accept = std::thread::Builder::new()
        .name("odl-accept".into())
        .spawn(move || {
            for stream in listener.incoming() {
                if shared.quit.load(Ordering::SeqCst) {
                    break;
                }
                match stream {
                    Ok(s) => {
                        let mut list = r.lock().unwrap();
                        list.retain(|h| !h.is_finished());
                        if let Some(h) = connect(&shared, s) {
                            list.push(h);
                        }
                    }
                    Err(e) => crate::warn(&format!("accept: {e}")),
                }
            }
        })?;
    Ok(Server { path, accept: Some(accept), readers })
}

fn connect(shared: &Arc<Shared>, stream: UnixStream) -> Option<JoinHandle<()>> {
    let id = shared.next_id.fetch_add(1, Ordering::SeqCst) + 1;
    let writer = match stream.try_clone() {
        Ok(w) => w,
        Err(e) => {
            crate::warn(&format!("client {id}: {e}"));
            return None;
        }
    };
    let _ = writer.set_write_timeout(Some(Duration::from_secs(2)));
    if let Some(old) = shared.client.lock().unwrap().replace(Client { id, stream: writer }) {
        crate::info(&format!("client {id} replaces client {}", old.id));
        let _ = old.stream.shutdown(std::net::Shutdown::Both);
    } else {
        crate::info(&format!("client {id} connected"));
    }
    let shared = shared.clone();
    std::thread::Builder::new()
        .name(format!("odl-client-{id}"))
        .spawn(move || read_loop(&shared, id, stream))
        .ok()
}

fn is_current(shared: &Shared, id: u64) -> bool {
    shared.client.lock().unwrap().as_ref().is_some_and(|c| c.id == id)
}

fn read_loop(shared: &Shared, id: u64, mut stream: UnixStream) {
    loop {
        let msg = match odl_proto::read_client(&mut stream) {
            Ok(Some(m)) => m,
            Ok(None) => break,
            Err(e) => {
                if !shared.quit.load(Ordering::SeqCst) {
                    crate::warn(&format!("client {id}: {e}"));
                }
                break;
            }
        };
        if !is_current(shared, id) {
            break;
        }
        match msg {
            ClientMsg::Start { codec, bitrate_kbps } => {
                let Some(codec) = Codec::parse(&codec) else {
                    shared.send(&ServerMsg::Error { message: format!("unsupported codec {codec}") });
                    continue;
                };
                crate::info(&format!("client {id}: start {} at {bitrate_kbps} kbps", codec.name()));
                *shared.request.lock().unwrap() = Some(Request { codec });
                shared.bitrate_kbps.store(bitrate_kbps, Ordering::SeqCst);
                shared.keyframe.store(false, Ordering::SeqCst);
                output::queue(Action::Start);
            }
            ClientMsg::KeyframeRequest => shared.keyframe.store(true, Ordering::SeqCst),
            ClientMsg::SetBitrate { kbps } => shared.bitrate_kbps.store(kbps, Ordering::SeqCst),
            ClientMsg::Stop => output::queue(Action::Stop),
        }
    }
    let mut guard = shared.client.lock().unwrap();
    if guard.as_ref().is_some_and(|c| c.id == id) {
        *guard = None;
        drop(guard);
        crate::info(&format!("client {id} disconnected"));
        output::queue(Action::Stop);
    }
}

impl Server {
    /// Joins every thread the server started, so none runs after OBS unloads the module.
    pub fn shutdown(&mut self, shared: &Shared) {
        shared.quit.store(true, Ordering::SeqCst);
        if let Some(c) = shared.client.lock().unwrap().take() {
            let _ = c.stream.shutdown(std::net::Shutdown::Both);
        }
        // Wakes accept() so the loop sees `quit`.
        let _ = UnixStream::connect(&self.path);
        if let Some(h) = self.accept.take() {
            let _ = h.join();
        }
        for h in self.readers.lock().unwrap().drain(..) {
            let _ = h.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}
