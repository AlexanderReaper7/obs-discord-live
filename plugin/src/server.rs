//! The socket the client connects to (the settings' address), and the state the OBS side shares with it.

use std::io;
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use odl_proto::{Addr, ClientMsg, ServerMsg, Stream};

use crate::encoder::Codec;
use crate::output::{self, Action};

pub struct Client {
    id: u64,
    stream: Stream,
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
    pub fn has_client(&self) -> bool {
        self.client.lock().unwrap().is_some()
    }

    /// Sends to the current client. A failed write drops the client and stops
    /// the output, the same as a disconnect.
    pub fn send(&self, msg: &ServerMsg) {
        let mut guard = self.client.lock().unwrap();
        let Some(client) = guard.as_mut() else { return };
        if let Err(e) = odl_proto::write(&mut client.stream, &msg.encode()) {
            crate::warn(&format!("client {}: write: {e}; dropping it", client.id));
            let _ = client.stream.shutdown();
            *guard = None;
            drop(guard);
            output::queue(Action::Stop);
        }
    }
}

enum Listener {
    #[cfg(unix)]
    Unix(UnixListener),
    Tcp(TcpListener),
}

impl Listener {
    fn bind(addr: &Addr) -> io::Result<Listener> {
        match addr {
            #[cfg(unix)]
            Addr::Unix(path) => {
                match std::fs::remove_file(path) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                }
                let l = UnixListener::bind(path)?;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
                Ok(Listener::Unix(l))
            }
            #[cfg(not(unix))]
            Addr::Unix(_) => Err(io::Error::new(io::ErrorKind::Unsupported, "Unix sockets need Unix")),
            Addr::Tcp(a) => TcpListener::bind(a.as_str()).map(Listener::Tcp),
        }
    }

    /// The stream, and the peer's address for TCP.
    fn accept(&self) -> io::Result<(Stream, Option<std::net::SocketAddr>)> {
        match self {
            #[cfg(unix)]
            Listener::Unix(l) => l.accept().map(|(s, _)| (Stream::Unix(s), None)),
            Listener::Tcp(l) => {
                let (s, peer) = l.accept()?;
                Stream::tcp(s).map(|s| (s, Some(peer)))
            }
        }
    }

    /// Where a connection reaches this listener, to wake `accept` at shutdown.
    fn wake_addr(&self, addr: &Addr) -> Addr {
        match self {
            #[cfg(unix)]
            Listener::Unix(_) => addr.clone(),
            Listener::Tcp(l) => match l.local_addr() {
                Ok(a) if a.ip().is_unspecified() => Addr::Tcp(format!("127.0.0.1:{}", a.port())),
                Ok(a) => Addr::Tcp(a.to_string()),
                Err(_) => addr.clone(),
            },
        }
    }
}

pub struct Server {
    addr: Addr,
    wake: Addr,
    accept: Option<JoinHandle<()>>,
    readers: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

pub fn spawn(shared: Arc<Shared>, addr: Addr) -> io::Result<Server> {
    let listener = Listener::bind(&addr).map_err(|e| io::Error::new(e.kind(), format!("{addr}: {e}")))?;
    let wake = listener.wake_addr(&addr);
    crate::info(&format!("listening on {addr}"));
    // A restart reuses the Shared a shutdown set this on.
    shared.quit.store(false, Ordering::SeqCst);
    let readers: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::default();
    let r = readers.clone();
    let accept = std::thread::Builder::new()
        .name("odl-accept".into())
        .spawn(move || {
            loop {
                let stream = listener.accept();
                if shared.quit.load(Ordering::SeqCst) {
                    break;
                }
                match stream {
                    Ok((s, peer)) => {
                        if let Some(peer) = peer {
                            crate::info(&format!("connection from {peer}"));
                        }
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
    Ok(Server { addr, wake, accept: Some(accept), readers })
}

fn connect(shared: &Arc<Shared>, stream: Stream) -> Option<JoinHandle<()>> {
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
        let _ = old.stream.shutdown();
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

fn read_loop(shared: &Shared, id: u64, mut stream: Stream) {
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
    pub fn addr(&self) -> &Addr {
        &self.addr
    }

    /// Joins every thread the server started, so none runs after OBS unloads the module.
    pub fn shutdown(&mut self, shared: &Shared) {
        shared.quit.store(true, Ordering::SeqCst);
        if let Some(c) = shared.client.lock().unwrap().take() {
            let _ = c.stream.shutdown();
        }
        // Wakes accept() so the loop sees `quit`.
        let _ = Stream::connect(&self.wake);
        if let Some(h) = self.accept.take() {
            let _ = h.join();
        }
        for h in self.readers.lock().unwrap().drain(..) {
            let _ = h.join();
        }
        if let Addr::Unix(path) = &self.addr {
            let _ = std::fs::remove_file(path);
        }
    }
}
