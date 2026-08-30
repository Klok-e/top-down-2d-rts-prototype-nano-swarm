use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
};

#[cfg(unix)]
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
        net::{UnixListener, UnixStream},
    },
    path::Path,
    sync::mpsc::RecvTimeoutError,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use bevy::{
    camera::RenderTarget,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
};
use serde::{Deserialize, Serialize};

use crate::{
    MainCamera,
    fly_camera::{CameraControlError, CameraZoom2d, FlyCamera2d, set_camera_view},
    intent::{BrushSelection, IntentGrid, IntentKind},
    nanobot::{
        Health, MatchOutcome, Nanobot, NanobotType, OpponentSwarm, OwnerSwarm, PopulationDemand,
        ProductionCollapseState, ProductionFacility, ProductionPriority, Swarm, SwarmId,
        SwarmMember,
    },
    resources::{ResourceKind, ResourceLedger},
    ui::intent_layer_panel::{IntentLayerButton, IntentLayerPanelRoot},
    zones::{PlayerIntentAction, PlayerIntentError, apply_player_intent},
};

pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_WAIT_DELTA: u64 = 18_000;
pub const DEFAULT_STATE_CELL_LIMIT: u32 = 10_000;
pub const MAX_STATE_CELL_LIMIT: u32 = 10_000;
#[cfg(unix)]
const MAX_CONTROL_RESPONSE_DURATION: Duration = Duration::from_secs(310);
#[cfg(unix)]
const CONTROL_RESPONSE_POLL_DURATION: Duration = Duration::from_millis(10);
#[cfg(unix)]
const MAX_CONTROL_LINE_DURATION: Duration = Duration::from_secs(30);
pub const AGENT_PROTOCOL_VERSION: u32 = 1;
pub const SUPPORTED_METHODS: [&str; 11] = [
    "session.hello",
    "state.get",
    "button.press",
    "intent.select",
    "map.apply",
    "camera.set",
    "camera.pan",
    "production_priority.set",
    "frame.wait",
    "screenshot.capture",
    "process.shutdown",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(u64),
    String(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolIntent {
    Gather,
    Build,
    Defend,
    Corridor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolMapAction {
    Paint,
    Erase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolButton {
    #[serde(rename = "intent.gather")]
    IntentGather,
    #[serde(rename = "intent.build")]
    IntentBuild,
    #[serde(rename = "intent.defend")]
    IntentDefend,
    #[serde(rename = "intent.corridor")]
    IntentCorridor,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentCommand {
    SessionHello,
    StateGet {
        cell_offset: u32,
        cell_limit: u32,
        map_revision: Option<u64>,
    },
    ButtonPress {
        button: ProtocolButton,
    },
    CameraSet {
        x: f32,
        y: f32,
        zoom: Option<f32>,
    },
    CameraPan {
        dx: f32,
        dy: f32,
    },
    FrameWait {
        frames: u64,
        fixed_ticks: u64,
    },
    ProductionPrioritySet {
        worker: u32,
        hauler: u32,
        defender: u32,
    },
    ProcessShutdown,
    ScreenshotCapture {
        name: Option<String>,
    },
    IntentSelect {
        intent: ProtocolIntent,
    },
    MapApply {
        action: ProtocolMapAction,
        intent: ProtocolIntent,
        x: i32,
        y: i32,
    },
}

impl AgentCommand {
    fn is_player_action(&self) -> bool {
        matches!(
            self,
            Self::ButtonPress { .. }
                | Self::IntentSelect { .. }
                | Self::MapApply { .. }
                | Self::ProductionPrioritySet { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentRequest {
    pub id: RequestId,
    pub command: AgentCommand,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentResponse {
    pub id: Option<RequestId>,
    pub ok: bool,
    pub frame: u64,
    pub fixed_tick: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AgentResponseError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentResponseError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentSubmitError {
    #[error("agent control request queue is full")]
    QueueFull,
    #[error("agent control request queue is disconnected")]
    Disconnected,
}

struct QueuedRequest {
    request: AgentRequest,
    reply: SyncSender<AgentResponse>,
    cancellation: RequestCancellation,
}

#[derive(Clone, Default)]
struct RequestCancellation(Arc<AtomicBool>);

impl RequestCancellation {
    fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

struct SubmittedRequest {
    response: Receiver<AgentResponse>,
    cancellation: RequestCancellation,
}

#[derive(Clone)]
pub struct AgentControlHandle {
    sender: SyncSender<QueuedRequest>,
    shared_clock: Arc<SharedAgentControlClock>,
}

impl AgentControlHandle {
    pub fn submit(
        &self,
        request: AgentRequest,
    ) -> Result<Receiver<AgentResponse>, AgentSubmitError> {
        self.submit_cancellable(request)
            .map(|submitted| submitted.response)
    }

    fn submit_cancellable(
        &self,
        request: AgentRequest,
    ) -> Result<SubmittedRequest, AgentSubmitError> {
        let (reply, response) = sync_channel(1);
        let cancellation = RequestCancellation::default();
        self.sender
            .try_send(QueuedRequest {
                request,
                reply,
                cancellation: cancellation.clone(),
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => AgentSubmitError::QueueFull,
                TrySendError::Disconnected(_) => AgentSubmitError::Disconnected,
            })?;
        Ok(SubmittedRequest {
            response,
            cancellation,
        })
    }

    fn clock(&self) -> AgentControlClock {
        AgentControlClock {
            frame: self.shared_clock.frame.load(Ordering::Acquire),
            fixed_tick: self.shared_clock.fixed_tick.load(Ordering::Acquire),
        }
    }
}

#[derive(Resource)]
struct AgentRequestInbox(Mutex<Receiver<QueuedRequest>>);

#[derive(Debug, Default, Resource)]
struct PendingButtonReleases(Vec<Entity>);

struct PendingWait {
    id: RequestId,
    reply: SyncSender<AgentResponse>,
    cancellation: RequestCancellation,
    start: AgentControlClock,
    target_frame: u64,
    target_fixed_tick: u64,
}

#[derive(Default, Resource)]
struct PendingWaits(Vec<PendingWait>);

#[derive(Resource)]
struct AgentScreenshotDirectory(PathBuf);

struct PendingScreenshot {
    id: RequestId,
    reply: SyncSender<AgentResponse>,
    cancellation: RequestCancellation,
    path: PathBuf,
    capture_clock: Option<AgentControlClock>,
}

#[derive(Default, Resource)]
struct PendingScreenshots(HashMap<Entity, PendingScreenshot>);

struct DeferredScreenshotRequest {
    id: RequestId,
    reply: SyncSender<AgentResponse>,
    cancellation: RequestCancellation,
    name: Option<String>,
}

#[derive(Default, Resource)]
struct DeferredScreenshotRequests(Vec<DeferredScreenshotRequest>);

#[derive(Default, Resource)]
struct ScreenshotSequence(u64);

#[derive(Debug, Default, Resource, Clone, Copy)]
pub struct AgentControlClock {
    pub frame: u64,
    pub fixed_tick: u64,
}

pub struct AgentControlCorePlugin {
    inbox: Mutex<Option<AgentRequestInbox>>,
    shared_clock: Arc<SharedAgentControlClock>,
}

#[derive(Default)]
struct SharedAgentControlClock {
    frame: AtomicU64,
    fixed_tick: AtomicU64,
}

#[derive(Resource, Clone)]
struct SharedAgentControlClockResource(Arc<SharedAgentControlClock>);

#[cfg(unix)]
#[derive(Debug, Clone)]
pub struct AgentControlConfig {
    pub socket_path: PathBuf,
    pub screenshot_directory: PathBuf,
}

#[cfg(unix)]
impl AgentControlConfig {
    pub fn at_socket_path(socket_path: PathBuf) -> Self {
        let screenshot_directory = socket_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("screenshots");
        Self {
            socket_path,
            screenshot_directory,
        }
    }

    pub fn from_environment() -> Result<Self, AgentControlServerError> {
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or(AgentControlServerError::MissingRuntimeDirectory)?;
        Ok(Self::at_socket_path(
            runtime_dir.join("nano-swarm/control.sock"),
        ))
    }
}

#[cfg(unix)]
#[derive(Debug, thiserror::Error)]
pub enum AgentControlServerError {
    #[error("XDG_RUNTIME_DIR is required for --agent-socket")]
    MissingRuntimeDirectory,
    #[error("control socket path has no parent directory: {0}")]
    MissingParent(PathBuf),
    #[error("control socket parent is not a secure directory: {0}")]
    UnsafeParent(PathBuf),
    #[error("refusing to replace a non-socket control path: {0}")]
    UnexpectedSocketPath(PathBuf),
    #[error("another nano-swarm control server is active at {0}")]
    SocketInUse(PathBuf),
    #[error("{action} {}: {source}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[cfg(unix)]
struct SocketFileGuard {
    path: PathBuf,
    device: u64,
    inode: u64,
}

#[cfg(unix)]
impl SocketFileGuard {
    fn new(path: PathBuf, metadata: &fs::Metadata) -> Self {
        Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[cfg(unix)]
impl Drop for SocketFileGuard {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.file_type().is_socket()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode
        }) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(unix)]
pub struct AgentControlPlugin {
    listener: Mutex<Option<UnixListener>>,
    socket_guard: Mutex<Option<SocketFileGuard>>,
    lifecycle_lock: Mutex<Option<File>>,
    screenshot_directory: PathBuf,
}

#[cfg(unix)]
impl AgentControlPlugin {
    pub fn bind(config: AgentControlConfig) -> Result<Self, AgentControlServerError> {
        prepare_control_socket_parent(&config.socket_path)?;
        let lifecycle_lock = acquire_control_socket_lock(&config.socket_path)?;
        remove_stale_control_socket(&config.socket_path)?;
        let listener = UnixListener::bind(&config.socket_path).map_err(|source| {
            AgentControlServerError::Io {
                action: "bind control socket",
                path: config.socket_path.clone(),
                source,
            }
        })?;
        let metadata = fs::symlink_metadata(&config.socket_path).map_err(|source| {
            let _ = fs::remove_file(&config.socket_path);
            AgentControlServerError::Io {
                action: "inspect bound control socket",
                path: config.socket_path.clone(),
                source,
            }
        })?;
        let socket_guard = SocketFileGuard::new(config.socket_path.clone(), &metadata);
        listener
            .set_nonblocking(true)
            .map_err(|source| AgentControlServerError::Io {
                action: "configure nonblocking control listener",
                path: config.socket_path.clone(),
                source,
            })?;
        fs::set_permissions(&config.socket_path, fs::Permissions::from_mode(0o600)).map_err(
            |source| AgentControlServerError::Io {
                action: "set control socket permissions",
                path: config.socket_path.clone(),
                source,
            },
        )?;
        Ok(Self {
            listener: Mutex::new(Some(listener)),
            socket_guard: Mutex::new(Some(socket_guard)),
            lifecycle_lock: Mutex::new(Some(lifecycle_lock)),
            screenshot_directory: config.screenshot_directory,
        })
    }
}

#[cfg(unix)]
impl Plugin for AgentControlPlugin {
    fn build(&self, app: &mut App) {
        let listener = self
            .listener
            .lock()
            .expect("agent control listener lock poisoned")
            .take()
            .expect("agent control plugin can only be added once");
        let socket_guard = self
            .socket_guard
            .lock()
            .expect("agent control socket guard lock poisoned")
            .take()
            .expect("agent control plugin can only be added once");
        let lifecycle_lock = self
            .lifecycle_lock
            .lock()
            .expect("agent control lifecycle lock poisoned")
            .take()
            .expect("agent control plugin can only be added once");
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (control, core_plugin) = AgentControlCorePlugin::channel(32);
        let worker = thread::Builder::new()
            .name("nano-swarm-agent-control".to_string())
            .spawn(move || run_control_socket_worker(listener, control, worker_stop))
            .expect("failed to spawn agent control worker");

        app.add_plugins(core_plugin)
            .insert_resource(AgentScreenshotDirectory(self.screenshot_directory.clone()))
            .insert_resource(AgentControlServer {
                stop,
                worker: Mutex::new(Some(worker)),
                _socket_guard: socket_guard,
                _lifecycle_lock: lifecycle_lock,
            });
    }
}

#[cfg(unix)]
#[derive(Resource)]
struct AgentControlServer {
    stop: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
    _socket_guard: SocketFileGuard,
    _lifecycle_lock: File,
}

#[cfg(unix)]
impl Drop for AgentControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self
            .worker
            .lock()
            .expect("agent control worker lock poisoned")
            .take()
        {
            let _ = worker.join();
        }
    }
}

#[cfg(unix)]
fn prepare_control_socket_parent(socket_path: &Path) -> Result<(), AgentControlServerError> {
    let parent = socket_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| AgentControlServerError::MissingParent(socket_path.to_path_buf()))?;
    match fs::symlink_metadata(parent) {
        Ok(_) => {}
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder
                .create(parent)
                .map_err(|source| AgentControlServerError::Io {
                    action: "create control socket directory",
                    path: parent.to_path_buf(),
                    source,
                })?;
        }
        Err(source) => {
            return Err(AgentControlServerError::Io {
                action: "inspect control socket directory",
                path: parent.to_path_buf(),
                source,
            });
        }
    }
    let metadata = fs::symlink_metadata(parent).map_err(|source| AgentControlServerError::Io {
        action: "inspect control socket directory",
        path: parent.to_path_buf(),
        source,
    })?;
    // SAFETY: `geteuid` has no preconditions and does not dereference pointers.
    let current_user = unsafe { libc::geteuid() };
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != current_user
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(AgentControlServerError::UnsafeParent(parent.to_path_buf()));
    }
    Ok(())
}

#[cfg(unix)]
fn control_socket_lock_path(socket_path: &Path) -> PathBuf {
    let mut lock_path = socket_path.as_os_str().to_os_string();
    lock_path.push(".lock");
    PathBuf::from(lock_path)
}

#[cfg(unix)]
fn acquire_control_socket_lock(socket_path: &Path) -> Result<File, AgentControlServerError> {
    let lock_path = control_socket_lock_path(socket_path);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&lock_path)
        .map_err(|source| AgentControlServerError::Io {
            action: "open control socket lifecycle lock",
            path: lock_path.clone(),
            source,
        })?;
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600)).map_err(|source| {
        AgentControlServerError::Io {
            action: "set control socket lifecycle lock permissions",
            path: lock_path.clone(),
            source,
        }
    })?;
    // SAFETY: `file` owns a valid descriptor and `flock` does not dereference pointers.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(file);
    }
    let source = io::Error::last_os_error();
    if source.kind() == io::ErrorKind::WouldBlock {
        Err(AgentControlServerError::SocketInUse(
            socket_path.to_path_buf(),
        ))
    } else {
        Err(AgentControlServerError::Io {
            action: "lock control socket lifecycle",
            path: lock_path,
            source,
        })
    }
}

#[cfg(unix)]
fn remove_stale_control_socket(socket_path: &Path) -> Result<(), AgentControlServerError> {
    let metadata = match fs::symlink_metadata(socket_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(AgentControlServerError::Io {
                action: "inspect control socket",
                path: socket_path.to_path_buf(),
                source,
            });
        }
    };
    if !metadata.file_type().is_socket() {
        return Err(AgentControlServerError::UnexpectedSocketPath(
            socket_path.to_path_buf(),
        ));
    }
    match UnixStream::connect(socket_path) {
        Ok(_) => Err(AgentControlServerError::SocketInUse(
            socket_path.to_path_buf(),
        )),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            ) =>
        {
            fs::remove_file(socket_path).map_err(|source| AgentControlServerError::Io {
                action: "remove stale control socket",
                path: socket_path.to_path_buf(),
                source,
            })
        }
        Err(source) => Err(AgentControlServerError::Io {
            action: "probe control socket",
            path: socket_path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(unix)]
fn run_control_socket_worker(
    listener: UnixListener,
    control: AgentControlHandle,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _address)) => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                serve_control_client(stream, &control, &stop);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

#[cfg(unix)]
fn serve_control_client(mut stream: UnixStream, control: &AgentControlHandle, stop: &AtomicBool) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader_stream);

    while !stop.load(Ordering::Acquire) {
        let line = match read_control_line(
            &mut reader,
            stop,
            Instant::now() + MAX_CONTROL_LINE_DURATION,
            Instant::now,
        ) {
            Ok(IncomingControlLine::Data(line)) => line,
            Ok(IncomingControlLine::TooLarge) => {
                let error = ProtocolError::RequestTooLarge {
                    limit: MAX_REQUEST_BYTES,
                };
                let response = AgentResponse::failure_with_optional_id(
                    None,
                    control.clock(),
                    error.response_code(),
                    error.to_string(),
                );
                if write_control_response(&mut stream, &response).is_err() {
                    return;
                }
                continue;
            }
            Ok(IncomingControlLine::Eof) | Err(_) => return,
        };
        let request = match parse_request_line(&line) {
            Ok(request) => request,
            Err(error) => {
                let response = AgentResponse::failure_with_optional_id(
                    extract_request_id(&line),
                    control.clock(),
                    error.response_code(),
                    error.to_string(),
                );
                if write_control_response(&mut stream, &response).is_err() {
                    return;
                }
                continue;
            }
        };
        let request_id = request.id.clone();
        let cancellable = matches!(
            &request.command,
            AgentCommand::FrameWait { .. } | AgentCommand::ScreenshotCapture { .. }
        );
        let submitted = match control.submit_cancellable(request) {
            Ok(submitted) => submitted,
            Err(error) => {
                let code = match error {
                    AgentSubmitError::QueueFull => "queue_full",
                    AgentSubmitError::Disconnected => "runtime_unavailable",
                };
                let response = AgentResponse::failure_with_optional_id(
                    Some(request_id),
                    control.clock(),
                    code,
                    error.to_string(),
                );
                if write_control_response(&mut stream, &response).is_err() {
                    return;
                }
                if matches!(error, AgentSubmitError::Disconnected) {
                    return;
                }
                continue;
            }
        };
        let response_started = Instant::now();
        let response = loop {
            match submitted
                .response
                .recv_timeout(CONTROL_RESPONSE_POLL_DURATION)
            {
                Ok(response) => break response,
                Err(RecvTimeoutError::Timeout) if stop.load(Ordering::Acquire) => return,
                Err(RecvTimeoutError::Timeout) if control_peer_disconnected(&stream) => {
                    if cancellable {
                        submitted.cancellation.cancel();
                    }
                    return;
                }
                Err(RecvTimeoutError::Timeout)
                    if response_started.elapsed() >= MAX_CONTROL_RESPONSE_DURATION =>
                {
                    if cancellable {
                        submitted.cancellation.cancel();
                    }
                    break AgentResponse::failure_with_optional_id(
                        Some(request_id),
                        control.clock(),
                        "request_timeout",
                        "control request exceeded the server response deadline",
                    );
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        if write_control_response(&mut stream, &response).is_err() {
            return;
        }
    }
}

#[cfg(unix)]
fn control_peer_disconnected(stream: &UnixStream) -> bool {
    let mut byte = 0u8;
    // SAFETY: the stream owns a valid descriptor and `byte` is writable for one byte.
    let result = unsafe {
        libc::recv(
            stream.as_raw_fd(),
            (&mut byte as *mut u8).cast(),
            1,
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    if result < 0
        && !matches!(
            io::Error::last_os_error().raw_os_error(),
            Some(code) if code == libc::EAGAIN || code == libc::EWOULDBLOCK || code == libc::EINTR
        )
    {
        return true;
    }

    let mut descriptor = libc::pollfd {
        fd: stream.as_raw_fd(),
        events: libc::POLLOUT,
        revents: 0,
    };
    // SAFETY: `descriptor` is valid for one entry for the duration of this call.
    let poll_result = unsafe { libc::poll(&mut descriptor, 1, 0) };
    if poll_result < 0 {
        return io::Error::last_os_error().kind() != io::ErrorKind::Interrupted;
    }
    poll_result > 0 && descriptor.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0
}

#[cfg(unix)]
enum IncomingControlLine {
    Data(Vec<u8>),
    TooLarge,
    Eof,
}

#[cfg(unix)]
fn read_control_line<R, N>(
    reader: &mut R,
    stop: &AtomicBool,
    deadline: Instant,
    mut now: N,
) -> io::Result<IncomingControlLine>
where
    R: BufRead,
    N: FnMut() -> Instant,
{
    let mut line = Vec::new();
    let mut too_large = false;
    loop {
        if now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "control request line exceeded its deadline",
            ));
        }
        let buffer = match reader.fill_buf() {
            Ok(buffer) => buffer,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) && !stop.load(Ordering::Acquire) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        if buffer.is_empty() {
            return Ok(if line.is_empty() && !too_large {
                IncomingControlLine::Eof
            } else if too_large {
                IncomingControlLine::TooLarge
            } else {
                IncomingControlLine::Data(line)
            });
        }

        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let data_length = newline.unwrap_or(buffer.len());
        if !too_large {
            if line.len().saturating_add(data_length) > MAX_REQUEST_BYTES {
                too_large = true;
                line.clear();
            } else {
                line.extend_from_slice(&buffer[..data_length]);
            }
        }
        let consumed = newline.map_or(buffer.len(), |position| position + 1);
        reader.consume(consumed);

        if newline.is_some() {
            if too_large {
                return Ok(IncomingControlLine::TooLarge);
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(IncomingControlLine::Data(line));
        }
    }
}

#[cfg(unix)]
fn extract_request_id(line: &[u8]) -> Option<RequestId> {
    serde_json::from_slice::<serde_json::Value>(line)
        .ok()?
        .get("id")
        .cloned()
        .and_then(|id| serde_json::from_value(id).ok())
}

#[cfg(unix)]
fn write_control_response(stream: &mut UnixStream, response: &AgentResponse) -> io::Result<()> {
    serde_json::to_writer(&mut *stream, response).map_err(io::Error::other)?;
    stream.write_all(b"\n")?;
    stream.flush()
}

impl AgentControlCorePlugin {
    pub fn channel(capacity: usize) -> (AgentControlHandle, Self) {
        let (sender, receiver) = sync_channel(capacity);
        let shared_clock = Arc::new(SharedAgentControlClock::default());
        (
            AgentControlHandle {
                sender,
                shared_clock: shared_clock.clone(),
            },
            Self {
                inbox: Mutex::new(Some(AgentRequestInbox(Mutex::new(receiver)))),
                shared_clock,
            },
        )
    }
}

impl Plugin for AgentControlCorePlugin {
    fn build(&self, app: &mut App) {
        let inbox = self
            .inbox
            .lock()
            .expect("agent control plugin inbox lock poisoned")
            .take()
            .expect("agent control core plugin can only be added once");
        app.insert_resource(inbox)
            .insert_resource(SharedAgentControlClockResource(self.shared_clock.clone()))
            .init_resource::<AgentControlClock>()
            .init_resource::<PendingButtonReleases>()
            .init_resource::<PendingWaits>()
            .init_resource::<PendingScreenshots>()
            .init_resource::<DeferredScreenshotRequests>()
            .init_resource::<ScreenshotSequence>()
            .add_observer(complete_agent_screenshot)
            .add_systems(
                PreUpdate,
                (
                    process_agent_cancellations,
                    queue_deferred_screenshots,
                    release_agent_buttons,
                    process_agent_requests,
                )
                    .chain()
                    .after(bevy::ui::UiSystems::Focus),
            )
            .add_systems(FixedLast, advance_agent_fixed_tick)
            .add_systems(
                Last,
                (
                    advance_agent_frame,
                    stamp_pending_screenshot_clocks,
                    complete_agent_waits,
                )
                    .chain(),
            );
    }
}

fn process_agent_requests(world: &mut World) {
    let requests = {
        let inbox = world.resource::<AgentRequestInbox>();
        inbox
            .0
            .lock()
            .expect("agent control request queue lock poisoned")
            .try_iter()
            .collect::<Vec<_>>()
    };

    for queued in requests {
        if queued.cancellation.is_cancelled() {
            continue;
        }
        let clock = *world.resource::<AgentControlClock>();
        if let AgentCommand::ScreenshotCapture { name } = &queued.request.command {
            queue_agent_screenshot(
                world,
                queued.request.id,
                queued.reply,
                queued.cancellation,
                name.clone(),
                clock,
            );
            continue;
        }
        if let AgentCommand::FrameWait {
            frames,
            fixed_ticks,
        } = &queued.request.command
        {
            if (*frames == 0 && *fixed_ticks == 0)
                || *frames > MAX_WAIT_DELTA
                || *fixed_ticks > MAX_WAIT_DELTA
            {
                let _ = queued.reply.send(AgentResponse::failure(
                    queued.request.id,
                    clock,
                    "invalid_parameters",
                    format!(
                        "frame.wait requires a positive delta no greater than {MAX_WAIT_DELTA}"
                    ),
                ));
                continue;
            }
            world.resource_mut::<PendingWaits>().0.push(PendingWait {
                id: queued.request.id,
                reply: queued.reply,
                cancellation: queued.cancellation,
                start: clock,
                target_frame: clock.frame.saturating_add(*frames),
                target_fixed_tick: clock.fixed_tick.saturating_add(*fixed_ticks),
            });
            continue;
        }
        let response = execute_agent_command(world, queued.request, clock);
        let _ = queued.reply.send(response);
    }
}

fn process_agent_cancellations(world: &mut World) {
    world
        .resource_mut::<PendingWaits>()
        .0
        .retain(|wait| !wait.cancellation.is_cancelled());
    world
        .resource_mut::<DeferredScreenshotRequests>()
        .0
        .retain(|request| !request.cancellation.is_cancelled());
    let screenshot_entities = world
        .resource::<PendingScreenshots>()
        .0
        .iter()
        .filter_map(|(entity, screenshot)| {
            screenshot.cancellation.is_cancelled().then_some(*entity)
        })
        .collect::<Vec<_>>();
    for entity in screenshot_entities {
        world.resource_mut::<PendingScreenshots>().0.remove(&entity);
        let _ = world.despawn(entity);
    }
}

fn queue_deferred_screenshots(world: &mut World) {
    let clock = *world.resource::<AgentControlClock>();
    if clock.frame == 0 {
        return;
    }
    let requests = std::mem::take(&mut world.resource_mut::<DeferredScreenshotRequests>().0);
    for request in requests {
        if request.cancellation.is_cancelled() {
            continue;
        }
        queue_agent_screenshot(
            world,
            request.id,
            request.reply,
            request.cancellation,
            request.name,
            clock,
        );
    }
}

fn execute_agent_command(
    world: &mut World,
    request: AgentRequest,
    clock: AgentControlClock,
) -> AgentResponse {
    if request.command.is_player_action()
        && world
            .get_resource::<MatchOutcome>()
            .is_some_and(|outcome| *outcome != MatchOutcome::InProgress)
    {
        return AgentResponse::failure(
            request.id,
            clock,
            "match_finished",
            "the match is already complete",
        );
    }
    let result = match request.command {
        AgentCommand::SessionHello => serde_json::json!({
            "protocol_version": AGENT_PROTOCOL_VERSION,
            "application": "nano-swarm",
            "methods": SUPPORTED_METHODS,
        }),
        AgentCommand::StateGet {
            cell_offset,
            cell_limit,
            map_revision,
        } => match collect_agent_state(world, cell_offset, cell_limit, map_revision) {
            Ok(state) => state,
            Err(current_revision) => {
                return AgentResponse::failure(
                    request.id,
                    clock,
                    "stale_state_page",
                    format!(
                        "state.get map_revision does not match current revision {current_revision}; restart at cell_offset 0"
                    ),
                );
            }
        },
        AgentCommand::ButtonPress { button } => {
            let kind = button.into();
            let candidates = {
                let mut buttons = world.query::<(Entity, &IntentLayerButton)>();
                buttons
                    .iter(world)
                    .filter_map(|(entity, marker)| (marker.kind == kind).then_some(entity))
                    .collect::<Vec<_>>()
            };
            let Some(entity) = candidates
                .into_iter()
                .find(|entity| is_intent_panel_descendant(world, *entity))
            else {
                return AgentResponse::failure(
                    request.id,
                    clock,
                    "button_unavailable",
                    "requested UI button is unavailable",
                );
            };
            world.entity_mut(entity).insert(Interaction::Pressed);
            world.resource_mut::<PendingButtonReleases>().0.push(entity);
            serde_json::json!({ "button": button })
        }
        AgentCommand::CameraSet { x, y, zoom } => {
            let camera_result = {
                let mut cameras = world.query_filtered::<(
                    &mut Transform,
                    &mut Projection,
                    &mut CameraZoom2d,
                    &mut FlyCamera2d,
                ), With<MainCamera>>();
                let Ok((mut transform, mut projection, mut camera_zoom, mut movement)) =
                    cameras.single_mut(world)
                else {
                    return AgentResponse::failure(
                        request.id,
                        clock,
                        "camera_unavailable",
                        "runtime must have exactly one main camera",
                    );
                };
                set_camera_view(
                    &mut transform,
                    &mut projection,
                    &mut camera_zoom,
                    &mut movement,
                    Vec2::new(x, y),
                    zoom,
                )
            };
            match camera_result {
                Ok(applied_zoom) => {
                    serde_json::json!({ "x": x, "y": y, "zoom": applied_zoom })
                }
                Err(CameraControlError::NonFinite) => {
                    return AgentResponse::failure(
                        request.id,
                        clock,
                        "invalid_camera",
                        CameraControlError::NonFinite.to_string(),
                    );
                }
                Err(CameraControlError::UnsupportedProjection) => {
                    return AgentResponse::failure(
                        request.id,
                        clock,
                        "camera_unavailable",
                        CameraControlError::UnsupportedProjection.to_string(),
                    );
                }
            }
        }
        AgentCommand::CameraPan { dx, dy } => {
            let camera_result = {
                let mut cameras = world.query_filtered::<(
                    &mut Transform,
                    &mut Projection,
                    &mut CameraZoom2d,
                    &mut FlyCamera2d,
                ), With<MainCamera>>();
                let Ok((mut transform, mut projection, mut camera_zoom, mut movement)) =
                    cameras.single_mut(world)
                else {
                    return AgentResponse::failure(
                        request.id,
                        clock,
                        "camera_unavailable",
                        "runtime must have exactly one main camera",
                    );
                };
                let position = transform.translation.truncate() + Vec2::new(dx, dy);
                set_camera_view(
                    &mut transform,
                    &mut projection,
                    &mut camera_zoom,
                    &mut movement,
                    position,
                    None,
                )
                .map(|zoom| (position, zoom))
            };
            match camera_result {
                Ok((position, zoom)) => {
                    serde_json::json!({ "x": position.x, "y": position.y, "zoom": zoom })
                }
                Err(CameraControlError::NonFinite) => {
                    return AgentResponse::failure(
                        request.id,
                        clock,
                        "invalid_camera",
                        CameraControlError::NonFinite.to_string(),
                    );
                }
                Err(CameraControlError::UnsupportedProjection) => {
                    return AgentResponse::failure(
                        request.id,
                        clock,
                        "camera_unavailable",
                        CameraControlError::UnsupportedProjection.to_string(),
                    );
                }
            }
        }
        AgentCommand::FrameWait { .. } => unreachable!("frame waits are queued before execution"),
        AgentCommand::IntentSelect { intent } => {
            let Some(mut selection) = world.get_resource_mut::<BrushSelection>() else {
                return AgentResponse::failure(
                    request.id,
                    clock,
                    "runtime_unavailable",
                    "BrushSelection resource is unavailable",
                );
            };
            selection.kind = intent.into();
            serde_json::json!({ "selected_intent": intent })
        }
        AgentCommand::MapApply {
            action,
            intent,
            x,
            y,
        } => {
            let outcome = world
                .get_resource::<MatchOutcome>()
                .copied()
                .unwrap_or_default();
            let Some(mut grid) = world.get_resource_mut::<IntentGrid>() else {
                return AgentResponse::failure(
                    request.id,
                    clock,
                    "runtime_unavailable",
                    "IntentGrid resource is unavailable",
                );
            };
            match apply_player_intent(
                &mut grid,
                outcome,
                IVec2::new(x, y),
                intent.into(),
                action.into(),
            ) {
                Ok(changed) => serde_json::json!({ "changed": changed }),
                Err(PlayerIntentError::MatchFinished) => {
                    return AgentResponse::failure(
                        request.id,
                        clock,
                        "match_finished",
                        PlayerIntentError::MatchFinished.to_string(),
                    );
                }
                Err(PlayerIntentError::OutOfBounds) => {
                    return AgentResponse::failure(
                        request.id,
                        clock,
                        "out_of_bounds",
                        PlayerIntentError::OutOfBounds.to_string(),
                    );
                }
            }
        }
        AgentCommand::ProductionPrioritySet {
            worker,
            hauler,
            defender,
        } => {
            let Some(mut priority) = world.get_resource_mut::<ProductionPriority>() else {
                return AgentResponse::failure(
                    request.id,
                    clock,
                    "runtime_unavailable",
                    "ProductionPriority resource is unavailable",
                );
            };
            if let Err(error) = priority.set_percentages(worker, hauler, defender) {
                return AgentResponse::failure(
                    request.id,
                    clock,
                    "invalid_priority",
                    error.to_string(),
                );
            }
            serde_json::json!({
                "worker": worker,
                "hauler": hauler,
                "defender": defender,
            })
        }
        AgentCommand::ProcessShutdown => {
            world.write_message(AppExit::Success);
            serde_json::json!({ "shutting_down": true })
        }
        AgentCommand::ScreenshotCapture { .. } => {
            unreachable!("screenshots are queued before execution")
        }
    };
    AgentResponse::success(request.id, clock, result)
}

fn collect_agent_state(
    world: &mut World,
    cell_offset: u32,
    cell_limit: u32,
    map_revision: Option<u64>,
) -> Result<serde_json::Value, u64> {
    let map = if let Some(grid) = world.get_resource::<IntentGrid>() {
        let current_revision = grid.revision();
        if map_revision.is_some_and(|revision| revision != current_revision)
            || (cell_offset > 0 && map_revision.is_none())
        {
            return Err(current_revision);
        }
        let active_cell_total = grid.iter_active_cells().count();
        let page_cells = grid
            .iter_active_cells()
            .skip(cell_offset as usize)
            .take(cell_limit as usize)
            .collect::<Vec<_>>();
        let active_cells = page_cells
            .iter()
            .map(|(cell, intents)| {
                let layers = intents
                    .iter_layers()
                    .map(|layer| {
                        serde_json::json!({
                            "intent": ProtocolIntent::from(layer.kind),
                            "owner": intents.owner(layer.kind).map(|owner| owner.0),
                        })
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({ "x": cell.x, "y": cell.y, "layers": layers })
            })
            .collect::<Vec<_>>();
        let next_cell_offset = (cell_offset as usize + active_cells.len() < active_cell_total)
            .then_some(cell_offset.saturating_add(active_cells.len() as u32));
        let defend_contests = page_cells
            .iter()
            .filter_map(|(cell, _)| {
                grid.defend_contest(*cell)
                    .map(|(incumbent, challenger)| (*cell, incumbent, challenger))
            })
            .map(|(cell, incumbent, challenger)| {
                serde_json::json!({
                    "x": cell.x,
                    "y": cell.y,
                    "incumbent": incumbent.0,
                    "challenger": challenger.0,
                })
            })
            .collect::<Vec<_>>();
        Some(serde_json::json!({
            "width": grid.width(),
            "height": grid.height(),
            "map_revision": current_revision,
            "active_cells": active_cells,
            "active_cell_total": active_cell_total,
            "next_cell_offset": next_cell_offset,
            "defend_contests": defend_contests,
        }))
    } else {
        None
    };
    if cell_offset > 0 {
        return Ok(serde_json::json!({ "map": map }));
    }

    let selected_intent = world
        .get_resource::<BrushSelection>()
        .map(|selection| ProtocolIntent::from(selection.kind));

    let camera = {
        let mut query = world.query_filtered::<(&Transform, &CameraZoom2d), With<MainCamera>>();
        query.single(world).ok().map(|(transform, zoom)| {
            serde_json::json!({
                "x": transform.translation.x,
                "y": transform.translation.y,
                "zoom": zoom.zoom,
            })
        })
    };
    let production_priority = world.get_resource::<ProductionPriority>().map(|priority| {
        serde_json::json!({
            "worker": priority.percentage(NanobotType::Worker),
            "hauler": priority.percentage(NanobotType::Hauler),
            "defender": priority.percentage(NanobotType::Defender),
        })
    });
    let outcome = world
        .get_resource::<MatchOutcome>()
        .copied()
        .unwrap_or_default();
    let collapse = world
        .get_resource::<ProductionCollapseState>()
        .copied()
        .unwrap_or_default();
    let match_state = serde_json::json!({
        "outcome": match_outcome_name(outcome),
        "player_collapsed": collapse.player_collapsed,
        "opponent_collapsed": collapse.opponent_collapsed,
    });

    let mut swarms = {
        let mut query =
            world.query_filtered::<(Entity, &SwarmId, Option<&OpponentSwarm>), With<Swarm>>();
        query
            .iter(world)
            .map(|(entity, id, opponent)| (entity, *id, opponent.is_some()))
            .collect::<Vec<_>>()
    };
    swarms.sort_by_key(|(_, id, _)| *id);
    let bots = {
        let mut query = world.query_filtered::<(
            &SwarmMember,
            &NanobotType,
            Option<&Health>,
            Option<&Transform>,
        ), With<Nanobot>>();
        query
            .iter(world)
            .map(|(member, kind, health, transform)| {
                (
                    member.0,
                    *kind,
                    health.map(|health| (health.current, health.max)),
                    transform.map(|transform| transform.translation.truncate()),
                )
            })
            .collect::<Vec<_>>()
    };
    let facilities = {
        let mut query = world.query::<(&ProductionFacility, Option<&OwnerSwarm>)>();
        query
            .iter(world)
            .map(|(facility, owner)| (owner.map(|owner| owner.0), facility.is_busy()))
            .collect::<Vec<_>>()
    };
    let ledger = world.get_resource::<ResourceLedger>().cloned();
    let demand = world
        .get_resource::<PopulationDemand>()
        .map(|demand| {
            swarms
                .iter()
                .map(|(_, id, _)| {
                    (
                        *id,
                        [
                            demand.desired_for(*id, NanobotType::Worker),
                            demand.desired_for(*id, NanobotType::Hauler),
                            demand.desired_for(*id, NanobotType::Defender),
                        ],
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let swarms = swarms
        .into_iter()
        .map(|(swarm_entity, id, opponent)| {
            let mut population = [0u32; NanobotType::COUNT];
            let mut health_current = 0u32;
            let mut health_max = 0u32;
            let mut position_sum = Vec2::ZERO;
            let mut positioned = 0u32;
            for (member, kind, health, position) in &bots {
                if *member != id {
                    continue;
                }
                population[nanobot_type_index(*kind)] += 1;
                if let Some((current, max)) = health {
                    health_current = health_current.saturating_add(*current);
                    health_max = health_max.saturating_add(*max);
                }
                if let Some(position) = position {
                    position_sum += *position;
                    positioned += 1;
                }
            }
            let centroid = (positioned > 0).then(|| {
                let position = position_sum / positioned as f32;
                serde_json::json!({ "x": position.x, "y": position.y })
            });
            let (facility_count, active_facilities) = facilities
                .iter()
                .filter(|(owner, _)| {
                    owner.is_some_and(|owner| owner == swarm_entity)
                        || (owner.is_none() && id.is_player())
                })
                .fold((0u32, 0u32), |(total, active), (_, busy)| {
                    (total + 1, active + u32::from(*busy))
                });
            let desired = |kind| {
                demand
                    .iter()
                    .find(|(demand_id, _)| *demand_id == id)
                    .map(|(_, desired)| desired[nanobot_type_index(kind)])
                    .unwrap_or_default()
            };

            serde_json::json!({
                "id": id.0,
                "opponent": opponent,
                "collapsed": if id.is_player() {
                    collapse.player_collapsed
                } else {
                    collapse.opponent_collapsed
                },
                "population": {
                    "worker": population[nanobot_type_index(NanobotType::Worker)],
                    "hauler": population[nanobot_type_index(NanobotType::Hauler)],
                    "defender": population[nanobot_type_index(NanobotType::Defender)],
                },
                "demand": {
                    "worker": desired(NanobotType::Worker),
                    "hauler": desired(NanobotType::Hauler),
                    "defender": desired(NanobotType::Defender),
                },
                "health": { "current": health_current, "max": health_max },
                "centroid": centroid,
                "minerals": ledger
                    .as_ref()
                    .map(|ledger| ledger.total_for(id, ResourceKind::Minerals))
                    .unwrap_or_default(),
                "facilities": { "total": facility_count, "active": active_facilities },
            })
        })
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "selected_intent": selected_intent,
        "map": map,
        "camera": camera,
        "production_priority": production_priority,
        "match": match_state,
        "swarms": swarms,
    }))
}

fn nanobot_type_index(kind: NanobotType) -> usize {
    match kind {
        NanobotType::Worker => 0,
        NanobotType::Hauler => 1,
        NanobotType::Defender => 2,
    }
}

fn match_outcome_name(outcome: MatchOutcome) -> &'static str {
    match outcome {
        MatchOutcome::InProgress => "in_progress",
        MatchOutcome::Victory => "victory",
        MatchOutcome::Defeat => "defeat",
    }
}

fn is_intent_panel_descendant(world: &World, entity: Entity) -> bool {
    let mut current = entity;
    loop {
        if world.get::<IntentLayerPanelRoot>(current).is_some() {
            return true;
        }
        let Some(parent) = world.get::<ChildOf>(current) else {
            return false;
        };
        current = parent.parent();
    }
}

fn release_agent_buttons(world: &mut World) {
    let entities = std::mem::take(&mut world.resource_mut::<PendingButtonReleases>().0);
    for entity in entities {
        if let Some(mut interaction) = world.get_mut::<Interaction>(entity) {
            *interaction = Interaction::None;
        }
    }
}

fn queue_agent_screenshot(
    world: &mut World,
    id: RequestId,
    reply: SyncSender<AgentResponse>,
    cancellation: RequestCancellation,
    name: Option<String>,
    clock: AgentControlClock,
) {
    if !world.resource::<PendingScreenshots>().0.is_empty()
        || !world.resource::<DeferredScreenshotRequests>().0.is_empty()
    {
        let _ = reply.send(AgentResponse::failure(
            id,
            clock,
            "screenshot_in_progress",
            "another screenshot capture is already in progress",
        ));
        return;
    }
    if clock.frame == 0 {
        world
            .resource_mut::<DeferredScreenshotRequests>()
            .0
            .push(DeferredScreenshotRequest {
                id,
                reply,
                cancellation,
                name,
            });
        return;
    }
    let target = {
        let mut cameras = world.query_filtered::<&RenderTarget, With<MainCamera>>();
        cameras.single(world).ok().cloned()
    };
    let Some(target) = target else {
        let _ = reply.send(AgentResponse::failure(
            id,
            clock,
            "camera_unavailable",
            "runtime must have exactly one main camera render target",
        ));
        return;
    };
    let path = match next_screenshot_path(world, name.as_deref()) {
        Ok(path) => path,
        Err((code, message)) => {
            let _ = reply.send(AgentResponse::failure(id, clock, code, message));
            return;
        }
    };

    let entity = world.spawn(Screenshot(target)).id();
    world.resource_mut::<PendingScreenshots>().0.insert(
        entity,
        PendingScreenshot {
            id,
            reply,
            cancellation,
            path,
            capture_clock: None,
        },
    );
}

fn stamp_pending_screenshot_clocks(
    clock: Res<AgentControlClock>,
    mut pending: ResMut<PendingScreenshots>,
) {
    for screenshot in pending.0.values_mut() {
        screenshot.capture_clock.get_or_insert(*clock);
    }
}

fn next_screenshot_path(
    world: &mut World,
    requested_name: Option<&str>,
) -> Result<PathBuf, (&'static str, String)> {
    let directory = world
        .get_resource::<AgentScreenshotDirectory>()
        .map(|directory| directory.0.clone())
        .ok_or_else(|| {
            (
                "runtime_unavailable",
                "screenshot directory is unavailable".to_string(),
            )
        })?;
    let directory = if directory.is_absolute() {
        directory
    } else {
        std::env::current_dir()
            .map_err(|error| ("screenshot_failed", error.to_string()))?
            .join(directory)
    };
    std::fs::create_dir_all(&directory)
        .map_err(|error| ("screenshot_failed", error.to_string()))?;
    #[cfg(unix)]
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| ("screenshot_failed", error.to_string()))?;

    let stem = match requested_name {
        Some(name)
            if !name.is_empty()
                && name.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                }) =>
        {
            name.to_string()
        }
        Some(_) => {
            return Err((
                "invalid_screenshot_name",
                "screenshot name may contain only ASCII letters, digits, '-' and '_'".to_string(),
            ));
        }
        None => "capture".to_string(),
    };
    let sequence = {
        let mut sequence = world.resource_mut::<ScreenshotSequence>();
        let current = sequence.0;
        sequence.0 = sequence.0.saturating_add(1);
        current
    };
    Ok(directory.join(format!("{stem}-{}-{sequence:06}.png", std::process::id())))
}

fn complete_agent_screenshot(
    capture: On<ScreenshotCaptured>,
    mut pending: ResMut<PendingScreenshots>,
    clock: Res<AgentControlClock>,
) {
    let Some(pending) = pending.0.remove(&capture.entity) else {
        return;
    };
    if pending.cancellation.is_cancelled() {
        return;
    }
    let capture_clock = pending.capture_clock.unwrap_or(*clock);
    let size = capture.image.texture_descriptor.size;
    let image = capture
        .image
        .clone()
        .try_into_dynamic()
        .map_err(|error| error.to_string())
        .map(|image| image.to_rgb8());
    if pending.cancellation.is_cancelled() {
        return;
    }
    let save_result = image
        .and_then(|image| image.save(&pending.path).map_err(|error| error.to_string()))
        .and_then(|()| {
            #[cfg(unix)]
            std::fs::set_permissions(&pending.path, std::fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
            Ok(())
        });
    if pending.cancellation.is_cancelled() {
        let _ = std::fs::remove_file(&pending.path);
        return;
    }
    let response = match save_result {
        Ok(()) => AgentResponse::success(
            pending.id,
            *clock,
            serde_json::json!({
                "path": pending.path.to_string_lossy(),
                "width": size.width,
                "height": size.height,
                "capture_frame": capture_clock.frame,
                "capture_fixed_tick": capture_clock.fixed_tick,
            }),
        ),
        Err(error) => AgentResponse::failure(
            pending.id,
            *clock,
            "screenshot_failed",
            format!("failed to save screenshot: {error}"),
        ),
    };
    let _ = pending.reply.send(response);
}

impl AgentResponse {
    fn success(id: RequestId, clock: AgentControlClock, result: serde_json::Value) -> Self {
        Self {
            id: Some(id),
            ok: true,
            frame: clock.frame,
            fixed_tick: clock.fixed_tick,
            result: Some(result),
            error: None,
        }
    }

    fn failure(
        id: RequestId,
        clock: AgentControlClock,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::failure_with_optional_id(Some(id), clock, code, message)
    }

    fn failure_with_optional_id(
        id: Option<RequestId>,
        clock: AgentControlClock,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id,
            ok: false,
            frame: clock.frame,
            fixed_tick: clock.fixed_tick,
            result: None,
            error: Some(AgentResponseError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

fn advance_agent_frame(
    mut clock: ResMut<AgentControlClock>,
    shared: Res<SharedAgentControlClockResource>,
) {
    clock.frame = clock.frame.saturating_add(1);
    shared.0.frame.store(clock.frame, Ordering::Release);
}

fn advance_agent_fixed_tick(
    mut clock: ResMut<AgentControlClock>,
    shared: Res<SharedAgentControlClockResource>,
) {
    clock.fixed_tick = clock.fixed_tick.saturating_add(1);
    shared
        .0
        .fixed_tick
        .store(clock.fixed_tick, Ordering::Release);
}

fn complete_agent_waits(clock: Res<AgentControlClock>, mut pending: ResMut<PendingWaits>) {
    let mut remaining = Vec::new();
    for wait in std::mem::take(&mut pending.0) {
        if wait.cancellation.is_cancelled() {
            continue;
        }
        if clock.frame < wait.target_frame || clock.fixed_tick < wait.target_fixed_tick {
            remaining.push(wait);
            continue;
        }
        let result = serde_json::json!({
            "waited_frames": clock.frame.saturating_sub(wait.start.frame),
            "waited_fixed_ticks": clock.fixed_tick.saturating_sub(wait.start.fixed_tick),
        });
        let _ = wait
            .reply
            .send(AgentResponse::success(wait.id, *clock, result));
    }
    pending.0 = remaining;
}

impl From<ProtocolIntent> for IntentKind {
    fn from(intent: ProtocolIntent) -> Self {
        match intent {
            ProtocolIntent::Gather => Self::Gather,
            ProtocolIntent::Build => Self::Build,
            ProtocolIntent::Defend => Self::Defend,
            ProtocolIntent::Corridor => Self::Corridor,
        }
    }
}

impl From<IntentKind> for ProtocolIntent {
    fn from(intent: IntentKind) -> Self {
        match intent {
            IntentKind::Gather => Self::Gather,
            IntentKind::Build => Self::Build,
            IntentKind::Defend => Self::Defend,
            IntentKind::Corridor => Self::Corridor,
        }
    }
}

impl From<ProtocolMapAction> for PlayerIntentAction {
    fn from(action: ProtocolMapAction) -> Self {
        match action {
            ProtocolMapAction::Paint => Self::Paint,
            ProtocolMapAction::Erase => Self::Erase,
        }
    }
}

impl From<ProtocolButton> for IntentKind {
    fn from(button: ProtocolButton) -> Self {
        match button {
            ProtocolButton::IntentGather => Self::Gather,
            ProtocolButton::IntentBuild => Self::Build,
            ProtocolButton::IntentDefend => Self::Defend,
            ProtocolButton::IntentCorridor => Self::Corridor,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("request exceeds {limit} byte limit")]
    RequestTooLarge { limit: usize },
    #[error("invalid JSON request: {0}")]
    InvalidJson(serde_json::Error),
    #[error("unknown method: {0}")]
    UnknownMethod(String),
    #[error("invalid request parameters: {0}")]
    InvalidParameters(String),
}

impl ProtocolError {
    fn response_code(&self) -> &'static str {
        match self {
            Self::RequestTooLarge { .. } => "request_too_large",
            Self::InvalidJson(_) => "invalid_request",
            Self::UnknownMethod(_) => "unknown_method",
            Self::InvalidParameters(_) => "invalid_parameters",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRequest {
    id: RequestId,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentSelectParams {
    intent: ProtocolIntent,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapApplyParams {
    action: ProtocolMapAction,
    intent: ProtocolIntent,
    x: i32,
    y: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ButtonPressParams {
    button: ProtocolButton,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CameraSetParams {
    x: f32,
    y: f32,
    #[serde(default)]
    zoom: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CameraPanParams {
    dx: f32,
    dy: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductionPrioritySetParams {
    worker: u32,
    hauler: u32,
    defender: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameWaitParams {
    #[serde(default)]
    frames: u64,
    #[serde(default)]
    fixed_ticks: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateGetParams {
    #[serde(default)]
    cell_offset: u32,
    #[serde(default = "default_state_cell_limit")]
    cell_limit: u32,
    #[serde(default)]
    map_revision: Option<u64>,
}

impl Default for StateGetParams {
    fn default() -> Self {
        Self {
            cell_offset: 0,
            cell_limit: DEFAULT_STATE_CELL_LIMIT,
            map_revision: None,
        }
    }
}

fn default_state_cell_limit() -> u32 {
    DEFAULT_STATE_CELL_LIMIT
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScreenshotCaptureParams {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyParams {}

fn require_empty_params(params: serde_json::Value) -> Result<(), ProtocolError> {
    if params.is_null() {
        return Ok(());
    }
    serde_json::from_value::<EmptyParams>(params)
        .map(|_| ())
        .map_err(ProtocolError::InvalidJson)
}

pub fn parse_request_line(line: &[u8]) -> Result<AgentRequest, ProtocolError> {
    if line.len() > MAX_REQUEST_BYTES {
        return Err(ProtocolError::RequestTooLarge {
            limit: MAX_REQUEST_BYTES,
        });
    }
    let request: WireRequest = serde_json::from_slice(line).map_err(ProtocolError::InvalidJson)?;
    let command = match request.method.as_str() {
        "session.hello" => {
            require_empty_params(request.params)?;
            AgentCommand::SessionHello
        }
        "state.get" => {
            let params = if request.params.is_null() {
                StateGetParams::default()
            } else {
                serde_json::from_value::<StateGetParams>(request.params)
                    .map_err(ProtocolError::InvalidJson)?
            };
            if params.cell_limit == 0 || params.cell_limit > MAX_STATE_CELL_LIMIT {
                return Err(ProtocolError::InvalidParameters(format!(
                    "state.get cell_limit must be between 1 and {MAX_STATE_CELL_LIMIT}"
                )));
            }
            AgentCommand::StateGet {
                cell_offset: params.cell_offset,
                cell_limit: params.cell_limit,
                map_revision: params.map_revision,
            }
        }
        "button.press" => {
            let params = serde_json::from_value::<ButtonPressParams>(request.params)
                .map_err(ProtocolError::InvalidJson)?;
            AgentCommand::ButtonPress {
                button: params.button,
            }
        }
        "camera.set" => {
            let params = serde_json::from_value::<CameraSetParams>(request.params)
                .map_err(ProtocolError::InvalidJson)?;
            AgentCommand::CameraSet {
                x: params.x,
                y: params.y,
                zoom: params.zoom,
            }
        }
        "camera.pan" => {
            let params = serde_json::from_value::<CameraPanParams>(request.params)
                .map_err(ProtocolError::InvalidJson)?;
            AgentCommand::CameraPan {
                dx: params.dx,
                dy: params.dy,
            }
        }
        "frame.wait" => {
            let params = serde_json::from_value::<FrameWaitParams>(request.params)
                .map_err(ProtocolError::InvalidJson)?;
            if params.frames == 0 && params.fixed_ticks == 0 {
                return Err(ProtocolError::InvalidParameters(
                    "frame.wait requires frames or fixed_ticks greater than zero".to_string(),
                ));
            }
            if params.frames > MAX_WAIT_DELTA || params.fixed_ticks > MAX_WAIT_DELTA {
                return Err(ProtocolError::InvalidParameters(format!(
                    "frame.wait values cannot exceed {MAX_WAIT_DELTA}"
                )));
            }
            AgentCommand::FrameWait {
                frames: params.frames,
                fixed_ticks: params.fixed_ticks,
            }
        }
        "intent.select" => {
            let params = serde_json::from_value::<IntentSelectParams>(request.params)
                .map_err(ProtocolError::InvalidJson)?;
            AgentCommand::IntentSelect {
                intent: params.intent,
            }
        }
        "map.apply" => {
            let params = serde_json::from_value::<MapApplyParams>(request.params)
                .map_err(ProtocolError::InvalidJson)?;
            AgentCommand::MapApply {
                action: params.action,
                intent: params.intent,
                x: params.x,
                y: params.y,
            }
        }
        "production_priority.set" => {
            let params = serde_json::from_value::<ProductionPrioritySetParams>(request.params)
                .map_err(ProtocolError::InvalidJson)?;
            AgentCommand::ProductionPrioritySet {
                worker: params.worker,
                hauler: params.hauler,
                defender: params.defender,
            }
        }
        "process.shutdown" => {
            require_empty_params(request.params)?;
            AgentCommand::ProcessShutdown
        }
        "screenshot.capture" => {
            let params = if request.params.is_null() {
                ScreenshotCaptureParams::default()
            } else {
                serde_json::from_value::<ScreenshotCaptureParams>(request.params)
                    .map_err(ProtocolError::InvalidJson)?
            };
            AgentCommand::ScreenshotCapture { name: params.name }
        }
        _ => return Err(ProtocolError::UnknownMethod(request.method)),
    };
    Ok(AgentRequest {
        id: request.id,
        command,
    })
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    #[cfg(unix)]
    fn remove_control_socket_test_directory(directory: &Path) {
        std::fs::remove_file(directory.join("control.sock.lock")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn parses_typed_intent_selection_request() {
        let request = parse_request_line(
            br#"{"id":7,"method":"intent.select","params":{"intent":"defend"}}"#,
        )
        .unwrap();

        assert_eq!(
            request,
            AgentRequest {
                id: RequestId::Number(7),
                command: AgentCommand::IntentSelect {
                    intent: ProtocolIntent::Defend,
                },
            }
        );
    }

    #[test]
    fn rejects_requests_over_the_ndjson_line_limit() {
        let oversized = vec![b' '; MAX_REQUEST_BYTES + 1];

        assert!(matches!(
            parse_request_line(&oversized),
            Err(ProtocolError::RequestTooLarge {
                limit: MAX_REQUEST_BYTES
            })
        ));
    }

    #[test]
    fn rejects_wait_deltas_above_the_server_bound() {
        let request = format!(
            "{{\"id\":9,\"method\":\"frame.wait\",\"params\":{{\"frames\":{}}}}}",
            MAX_WAIT_DELTA + 1
        );

        assert!(matches!(
            parse_request_line(request.as_bytes()),
            Err(ProtocolError::InvalidParameters(_))
        ));
    }

    #[test]
    fn parses_typed_map_edit_request() {
        let request = parse_request_line(
            br#"{"id":"paint-1","method":"map.apply","params":{"action":"paint","intent":"build","x":2,"y":-3}}"#,
        )
        .unwrap();

        assert_eq!(
            request,
            AgentRequest {
                id: RequestId::String("paint-1".to_string()),
                command: AgentCommand::MapApply {
                    action: ProtocolMapAction::Paint,
                    intent: ProtocolIntent::Build,
                    x: 2,
                    y: -3,
                },
            }
        );
    }

    #[test]
    fn intent_selection_command_changes_the_runtime_resource() {
        use crate::intent::{BrushSelection, IntentKind};
        use bevy::prelude::*;

        let mut app = App::new();
        app.init_resource::<BrushSelection>();
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);

        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(11),
                command: AgentCommand::IntentSelect {
                    intent: ProtocolIntent::Defend,
                },
            })
            .unwrap();
        app.update();
        let response = response.recv().unwrap();

        assert!(response.ok);
        assert_eq!(response.id, Some(RequestId::Number(11)));
        assert_eq!(response.frame, 0);
        assert_eq!(response.fixed_tick, 0);
        assert_eq!(
            app.world().resource::<BrushSelection>().kind,
            IntentKind::Defend
        );
    }

    #[test]
    fn map_command_uses_player_defend_contest_rules() {
        use crate::{
            intent::{IntentGrid, IntentKind},
            nanobot::{MatchOutcome, SwarmId},
        };
        use bevy::prelude::*;

        let cell = IVec2::new(2, -3);
        let opponent = SwarmId(4);
        let mut grid = IntentGrid::new(11, 11);
        grid.paint_owned(cell, IntentKind::Defend, Some(opponent));
        let mut app = App::new();
        app.insert_resource(grid)
            .insert_resource(MatchOutcome::InProgress);
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);

        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(12),
                command: AgentCommand::MapApply {
                    action: ProtocolMapAction::Paint,
                    intent: ProtocolIntent::Defend,
                    x: cell.x,
                    y: cell.y,
                },
            })
            .unwrap();
        app.update();
        let response = response.recv().unwrap();

        assert!(response.ok, "{:?}", response.error);
        assert_eq!(
            response.result,
            Some(serde_json::json!({ "changed": true }))
        );
        let grid = app.world().resource::<IntentGrid>();
        assert_eq!(grid.cell(cell).unwrap().owner(IntentKind::Defend), None);
        assert_eq!(
            grid.defend_contests(),
            vec![(cell, opponent, SwarmId::PLAYER)]
        );
    }

    #[test]
    fn parses_stable_button_id() {
        let request = parse_request_line(
            br#"{"id":13,"method":"button.press","params":{"button":"intent.defend"}}"#,
        )
        .unwrap();

        assert_eq!(
            request,
            AgentRequest {
                id: RequestId::Number(13),
                command: AgentCommand::ButtonPress {
                    button: ProtocolButton::IntentDefend,
                },
            }
        );
    }

    #[test]
    fn button_command_activates_and_releases_the_real_ui_button() {
        use crate::{
            intent::{BrushSelection, IntentKind},
            ui::intent_layer_panel::{
                IntentLayerButton, IntentLayerPanelRoot, intent_layer_button_click_system,
            },
        };
        use bevy::prelude::*;

        fn clear_interactions_like_headless_ui_focus(
            mut interactions: Query<&mut Interaction, With<IntentLayerButton>>,
        ) {
            for mut interaction in &mut interactions {
                *interaction = Interaction::None;
            }
        }

        let mut app = App::new();
        app.init_resource::<BrushSelection>()
            .add_systems(
                PreUpdate,
                clear_interactions_like_headless_ui_focus.in_set(bevy::ui::UiSystems::Focus),
            )
            .add_systems(Update, intent_layer_button_click_system);
        let root = app.world_mut().spawn(IntentLayerPanelRoot).id();
        let button = app
            .world_mut()
            .spawn((
                Button,
                IntentLayerButton {
                    kind: IntentKind::Defend,
                },
                Interaction::None,
            ))
            .id();
        app.world_mut().entity_mut(root).add_child(button);
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);

        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(14),
                command: AgentCommand::ButtonPress {
                    button: ProtocolButton::IntentDefend,
                },
            })
            .unwrap();
        app.update();
        let response = response.recv().unwrap();

        assert!(response.ok, "{:?}", response.error);
        assert_eq!(
            app.world().resource::<BrushSelection>().kind,
            IntentKind::Defend
        );
        assert_eq!(
            app.world().entity(button).get::<Interaction>(),
            Some(&Interaction::Pressed)
        );

        app.update();
        assert_eq!(
            app.world().entity(button).get::<Interaction>(),
            Some(&Interaction::None)
        );
    }

    #[test]
    fn parses_absolute_camera_request() {
        let request = parse_request_line(
            br#"{"id":15,"method":"camera.set","params":{"x":1024.0,"y":256.0,"zoom":2.0}}"#,
        )
        .unwrap();

        assert_eq!(
            request,
            AgentRequest {
                id: RequestId::Number(15),
                command: AgentCommand::CameraSet {
                    x: 1024.0,
                    y: 256.0,
                    zoom: Some(2.0),
                },
            }
        );
    }

    #[test]
    fn camera_command_synchronizes_position_projection_zoom_and_velocity() {
        use crate::{
            MainCamera,
            fly_camera::{CameraZoom2d, FlyCamera2d},
        };
        use bevy::prelude::*;

        let mut app = App::new();
        let camera = app
            .world_mut()
            .spawn((
                MainCamera,
                Transform::from_xyz(1.0, 2.0, 7.0),
                Projection::Orthographic(OrthographicProjection {
                    scale: 3.0,
                    ..OrthographicProjection::default_2d()
                }),
                CameraZoom2d {
                    zoom: 3.0,
                    zoom_min_max: (1.0, 10.0),
                    ..default()
                },
                FlyCamera2d {
                    velocity: Vec2::new(4.0, -2.0),
                    ..default()
                },
            ))
            .id();
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);

        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(16),
                command: AgentCommand::CameraSet {
                    x: 1024.0,
                    y: 256.0,
                    zoom: Some(2.0),
                },
            })
            .unwrap();
        app.update();
        let response = response.recv().unwrap();

        assert!(response.ok, "{:?}", response.error);
        let entity = app.world().entity(camera);
        let translation = entity.get::<Transform>().unwrap().translation;
        assert_abs_diff_eq!(translation.x, 1024.0, epsilon = 0.01);
        assert_abs_diff_eq!(translation.y, 256.0, epsilon = 0.01);
        assert_abs_diff_eq!(translation.z, 7.0, epsilon = 0.01);
        assert_abs_diff_eq!(
            entity.get::<CameraZoom2d>().unwrap().zoom,
            2.0,
            epsilon = 1e-5
        );
        let Projection::Orthographic(projection) = entity.get::<Projection>().unwrap() else {
            panic!("main camera must use an orthographic projection");
        };
        assert_abs_diff_eq!(projection.scale, 2.0, epsilon = 1e-5);
        let velocity = entity.get::<FlyCamera2d>().unwrap().velocity;
        assert_abs_diff_eq!(velocity.x, 0.0, epsilon = 1e-5);
        assert_abs_diff_eq!(velocity.y, 0.0, epsilon = 1e-5);
    }

    #[test]
    fn camera_pan_is_relative_and_preserves_zoom() {
        use crate::{
            MainCamera,
            fly_camera::{CameraZoom2d, FlyCamera2d},
        };
        use bevy::prelude::*;

        let mut app = App::new();
        let camera = app
            .world_mut()
            .spawn((
                MainCamera,
                Transform::from_xyz(10.0, 20.0, 3.0),
                Projection::Orthographic(OrthographicProjection {
                    scale: 4.0,
                    ..OrthographicProjection::default_2d()
                }),
                CameraZoom2d {
                    zoom: 4.0,
                    zoom_min_max: (1.0, 10.0),
                    ..default()
                },
                FlyCamera2d::default(),
            ))
            .id();
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);

        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(17),
                command: AgentCommand::CameraPan { dx: -3.0, dy: 8.0 },
            })
            .unwrap();
        app.update();
        assert!(response.recv().unwrap().ok);

        let entity = app.world().entity(camera);
        let translation = entity.get::<Transform>().unwrap().translation;
        assert_abs_diff_eq!(translation.x, 7.0, epsilon = 0.01);
        assert_abs_diff_eq!(translation.y, 28.0, epsilon = 0.01);
        assert_abs_diff_eq!(translation.z, 3.0, epsilon = 0.01);
        assert_abs_diff_eq!(
            entity.get::<CameraZoom2d>().unwrap().zoom,
            4.0,
            epsilon = 1e-5
        );
    }

    #[test]
    fn production_priority_command_rejects_invalid_percentages_without_mutation() {
        use crate::nanobot::{NanobotType, ProductionPriority};
        use bevy::prelude::*;

        let mut app = App::new();
        app.init_resource::<ProductionPriority>();
        let before = app.world().resource::<ProductionPriority>().weights.clone();
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);

        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(18),
                command: AgentCommand::ProductionPrioritySet {
                    worker: 50,
                    hauler: 25,
                    defender: 15,
                },
            })
            .unwrap();
        app.update();
        let response = response.recv().unwrap();

        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "invalid_priority");
        let priority = app.world().resource::<ProductionPriority>();
        assert_eq!(priority.weights, before);
        assert_eq!(priority.weight(NanobotType::Worker), 6);
    }

    #[test]
    fn production_priority_command_applies_valid_player_percentages() {
        use crate::nanobot::{NanobotType, ProductionPriority};
        use bevy::prelude::*;

        let mut app = App::new();
        app.init_resource::<ProductionPriority>();
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);
        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(181),
                command: AgentCommand::ProductionPrioritySet {
                    worker: 20,
                    hauler: 20,
                    defender: 60,
                },
            })
            .unwrap();

        app.update();
        assert!(response.recv().unwrap().ok);
        let priority = app.world().resource::<ProductionPriority>();
        assert_eq!(priority.weight(NanobotType::Worker), 20);
        assert_eq!(priority.weight(NanobotType::Hauler), 20);
        assert_eq!(priority.weight(NanobotType::Defender), 60);
    }

    #[test]
    fn completed_match_rejects_player_action_commands() {
        use crate::{
            intent::{BrushSelection, IntentKind},
            nanobot::MatchOutcome,
        };
        use bevy::prelude::*;

        let mut app = App::new();
        app.init_resource::<BrushSelection>()
            .insert_resource(MatchOutcome::Victory);
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);

        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(19),
                command: AgentCommand::IntentSelect {
                    intent: ProtocolIntent::Defend,
                },
            })
            .unwrap();
        app.update();
        let response = response.recv().unwrap();

        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "match_finished");
        assert_eq!(
            app.world().resource::<BrushSelection>().kind,
            IntentKind::Gather
        );
    }

    #[test]
    fn session_hello_reports_protocol_and_supported_methods() {
        use bevy::prelude::*;

        let mut app = App::new();
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);
        let response = control
            .submit(parse_request_line(br#"{"id":20,"method":"session.hello"}"#).unwrap())
            .unwrap();

        app.update();
        let response = response.recv().unwrap();

        assert!(response.ok);
        let result = response.result.unwrap();
        assert_eq!(result["protocol_version"], 1);
        assert!(
            result["methods"]
                .as_array()
                .unwrap()
                .iter()
                .any(|method| method == "state.get"),
            "hello must advertise state.get"
        );
    }

    #[test]
    fn state_get_returns_sparse_owned_intent_state() {
        use crate::{
            intent::{BrushSelection, IntentGrid, IntentKind},
            nanobot::SwarmId,
        };
        use bevy::prelude::*;

        let mut grid = IntentGrid::new(7, 5);
        grid.paint_owned(IVec2::new(-2, 1), IntentKind::Build, Some(SwarmId::PLAYER));
        let mut app = App::new();
        app.insert_resource(grid).insert_resource(BrushSelection {
            kind: IntentKind::Build,
        });
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);

        let response = control
            .submit(parse_request_line(br#"{"id":21,"method":"state.get"}"#).unwrap())
            .unwrap();
        app.update();
        let response = response.recv().unwrap();

        assert!(response.ok, "{:?}", response.error);
        let state = response.result.unwrap();
        assert_eq!(state["selected_intent"], "build");
        assert_eq!(state["map"]["width"], 7);
        assert_eq!(state["map"]["height"], 5);
        assert_eq!(state["map"]["active_cells"].as_array().unwrap().len(), 1);
        assert_eq!(state["map"]["active_cells"][0]["x"], -2);
        assert_eq!(state["map"]["active_cells"][0]["y"], 1);
        assert_eq!(
            state["map"]["active_cells"][0]["layers"][0],
            serde_json::json!({ "intent": "build", "owner": 0 })
        );
    }

    #[test]
    fn state_get_paginates_active_cells_with_a_bounded_page() {
        use crate::intent::{IntentGrid, IntentKind};
        use bevy::prelude::*;

        let mut grid = IntentGrid::new(7, 5);
        for x in -1..=1 {
            grid.paint_owned(IVec2::new(x, 0), IntentKind::Gather, Some(SwarmId::PLAYER));
        }
        let mut app = App::new();
        app.insert_resource(grid);
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);
        let first = control
            .submit(AgentRequest {
                id: RequestId::Number(211),
                command: AgentCommand::StateGet {
                    cell_offset: 0,
                    cell_limit: 2,
                    map_revision: None,
                },
            })
            .unwrap();
        app.update();
        let first = first.recv().unwrap().result.unwrap();

        assert_eq!(first["map"]["active_cell_total"], 3);
        assert_eq!(first["map"]["active_cells"].as_array().unwrap().len(), 2);
        assert_eq!(first["map"]["next_cell_offset"], 2);
        let map_revision = first["map"]["map_revision"].as_u64().unwrap();

        let second = control
            .submit(AgentRequest {
                id: RequestId::Number(212),
                command: AgentCommand::StateGet {
                    cell_offset: 2,
                    cell_limit: 2,
                    map_revision: Some(map_revision),
                },
            })
            .unwrap();
        app.update();
        let second = second.recv().unwrap().result.unwrap();
        assert_eq!(second["map"]["active_cells"].as_array().unwrap().len(), 1);
        assert!(second["map"]["next_cell_offset"].is_null());
    }

    #[test]
    fn state_get_continuation_pages_do_not_mix_newer_non_map_state() {
        use crate::{
            intent::{IntentGrid, IntentKind},
            nanobot::MatchOutcome,
        };
        use bevy::prelude::*;

        let mut grid = IntentGrid::new(7, 5);
        grid.paint_owned(IVec2::new(0, 0), IntentKind::Gather, Some(SwarmId::PLAYER));
        grid.paint_owned(IVec2::new(1, 0), IntentKind::Gather, Some(SwarmId::PLAYER));
        let mut app = App::new();
        app.insert_resource(grid)
            .insert_resource(MatchOutcome::InProgress);
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);

        let first = control
            .submit(AgentRequest {
                id: RequestId::Number(216),
                command: AgentCommand::StateGet {
                    cell_offset: 0,
                    cell_limit: 1,
                    map_revision: None,
                },
            })
            .unwrap();
        app.update();
        let first = first.recv().unwrap().result.unwrap();
        let map_revision = first["map"]["map_revision"].as_u64().unwrap();
        assert_eq!(first["match"]["outcome"], "in_progress");

        app.world_mut().insert_resource(MatchOutcome::Victory);
        let second = control
            .submit(AgentRequest {
                id: RequestId::Number(217),
                command: AgentCommand::StateGet {
                    cell_offset: 1,
                    cell_limit: 1,
                    map_revision: Some(map_revision),
                },
            })
            .unwrap();
        app.update();
        let second = second.recv().unwrap().result.unwrap();

        assert_eq!(second.as_object().unwrap().len(), 1);
        assert!(second.get("map").is_some());
        assert!(second.get("match").is_none());
    }

    #[test]
    fn state_get_rejects_a_page_from_a_changed_map_revision() {
        use crate::intent::{IntentGrid, IntentKind};
        use bevy::prelude::*;

        let mut grid = IntentGrid::new(7, 5);
        grid.paint_owned(IVec2::new(0, 0), IntentKind::Gather, Some(SwarmId::PLAYER));
        grid.paint_owned(IVec2::new(1, 0), IntentKind::Gather, Some(SwarmId::PLAYER));
        let mut app = App::new();
        app.insert_resource(grid);
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);
        let first = control
            .submit(AgentRequest {
                id: RequestId::Number(213),
                command: AgentCommand::StateGet {
                    cell_offset: 0,
                    cell_limit: 1,
                    map_revision: None,
                },
            })
            .unwrap();
        app.update();
        let revision = first.recv().unwrap().result.unwrap()["map"]["map_revision"]
            .as_u64()
            .unwrap();
        app.world_mut().resource_mut::<IntentGrid>().paint_owned(
            IVec2::new(-1, 0),
            IntentKind::Gather,
            Some(SwarmId::PLAYER),
        );

        let stale = control
            .submit(AgentRequest {
                id: RequestId::Number(214),
                command: AgentCommand::StateGet {
                    cell_offset: 1,
                    cell_limit: 1,
                    map_revision: Some(revision),
                },
            })
            .unwrap();
        app.update();
        let stale = stale.recv().unwrap();

        assert!(!stale.ok);
        assert_eq!(stale.error.unwrap().code, "stale_state_page");
    }

    #[test]
    fn state_get_bounds_defend_contests_to_the_active_cell_page() {
        use crate::intent::{IntentGrid, IntentKind};
        use bevy::prelude::*;

        let mut grid = IntentGrid::new(7, 5);
        for x in -1..=1 {
            let cell = IVec2::new(x, 0);
            grid.paint_owned(cell, IntentKind::Defend, Some(SwarmId(7)));
            grid.contest_defend(cell, SwarmId::PLAYER);
        }
        let mut app = App::new();
        app.insert_resource(grid);
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);
        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(215),
                command: AgentCommand::StateGet {
                    cell_offset: 0,
                    cell_limit: 1,
                    map_revision: None,
                },
            })
            .unwrap();
        app.update();
        let state = response.recv().unwrap().result.unwrap();

        assert_eq!(state["map"]["active_cells"].as_array().unwrap().len(), 1);
        assert_eq!(state["map"]["defend_contests"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn state_get_reports_camera_economy_outcome_and_swarm_status() {
        use crate::{
            MainCamera,
            fly_camera::CameraZoom2d,
            nanobot::{
                Health, MatchOutcome, Nanobot, NanobotType, ProductionCollapseState,
                ProductionPriority, Swarm, SwarmId, SwarmMember,
            },
            resources::{ResourceKind, ResourceLedger},
        };
        use bevy::prelude::*;

        let mut ledger = ResourceLedger::new();
        ledger.add_for(SwarmId::PLAYER, ResourceKind::Minerals, 37);
        let mut app = App::new();
        app.init_resource::<ProductionPriority>()
            .insert_resource(ledger)
            .insert_resource(MatchOutcome::Victory)
            .insert_resource(ProductionCollapseState {
                player_collapsed: false,
                opponent_collapsed: true,
            });
        app.world_mut().spawn((
            MainCamera,
            Transform::from_xyz(80.0, -40.0, 0.0),
            CameraZoom2d {
                zoom: 2.5,
                ..default()
            },
        ));
        app.world_mut().spawn((Swarm::default(), SwarmId::PLAYER));
        app.world_mut().spawn((
            Nanobot::default(),
            NanobotType::Worker,
            SwarmMember::new(SwarmId::PLAYER),
            Health::full(100),
            Transform::from_xyz(5.0, 7.0, 0.0),
        ));
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);

        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(22),
                command: AgentCommand::StateGet {
                    cell_offset: 0,
                    cell_limit: DEFAULT_STATE_CELL_LIMIT,
                    map_revision: None,
                },
            })
            .unwrap();
        app.update();
        let state = response.recv().unwrap().result.unwrap();

        assert_eq!(
            state["camera"],
            serde_json::json!({ "x": 80.0, "y": -40.0, "zoom": 2.5 })
        );
        assert_eq!(state["production_priority"]["worker"], 60);
        assert_eq!(state["match"]["outcome"], "victory");
        assert_eq!(state["match"]["opponent_collapsed"], true);
        assert_eq!(state["swarms"][0]["id"], 0);
        assert_eq!(state["swarms"][0]["population"]["worker"], 1);
        assert_eq!(state["swarms"][0]["minerals"], 37);
        assert_eq!(
            state["swarms"][0]["centroid"],
            serde_json::json!({ "x": 5.0, "y": 7.0 })
        );
    }

    #[test]
    fn frame_wait_responds_only_after_requested_frames_complete() {
        use std::sync::mpsc::TryRecvError;

        use bevy::prelude::*;

        let mut app = App::new();
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);
        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(23),
                command: AgentCommand::FrameWait {
                    frames: 2,
                    fixed_ticks: 0,
                },
            })
            .unwrap();

        app.update();
        assert!(matches!(response.try_recv(), Err(TryRecvError::Empty)));
        app.update();
        let response = response.recv().unwrap();

        assert!(response.ok);
        assert_eq!(response.frame, 2);
        assert_eq!(response.fixed_tick, 0);
    }

    #[test]
    fn cancellation_targets_one_request_instance_when_protocol_ids_repeat() {
        use bevy::prelude::*;

        let mut app = App::new();
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);
        let cancelled = control
            .submit_cancellable(AgentRequest {
                id: RequestId::Number(23),
                command: AgentCommand::FrameWait {
                    frames: 100,
                    fixed_ticks: 0,
                },
            })
            .unwrap();
        let retained = control
            .submit(AgentRequest {
                id: RequestId::Number(23),
                command: AgentCommand::SessionHello,
            })
            .unwrap();
        cancelled.cancellation.cancel();

        app.update();

        assert!(cancelled.response.recv().is_err());
        assert!(retained.recv().unwrap().ok);
    }

    #[test]
    fn wait_completion_observes_cancellation_from_the_same_frame() {
        use bevy::prelude::*;

        #[derive(Resource)]
        struct CancelDuringUpdate(RequestCancellation);

        fn cancel_during_update(cancellation: Res<CancelDuringUpdate>) {
            cancellation.0.cancel();
        }

        let mut app = App::new();
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin)
            .add_systems(Update, cancel_during_update);
        let submitted = control
            .submit_cancellable(AgentRequest {
                id: RequestId::Number(232),
                command: AgentCommand::FrameWait {
                    frames: 1,
                    fixed_ticks: 0,
                },
            })
            .unwrap();
        app.insert_resource(CancelDuringUpdate(submitted.cancellation));

        app.update();

        assert!(submitted.response.recv().is_err());
    }

    #[test]
    fn fixed_tick_wait_uses_the_fixed_simulation_clock() {
        use std::sync::mpsc::TryRecvError;

        use bevy::{prelude::*, time::TimeUpdateStrategy};

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(crate::fixed_simulation_time())
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1));
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);
        app.update();
        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(231),
                command: AgentCommand::FrameWait {
                    frames: 0,
                    fixed_ticks: 2,
                },
            })
            .unwrap();

        app.update();
        assert!(matches!(response.try_recv(), Err(TryRecvError::Empty)));
        app.update();
        let response = response.recv().unwrap();

        assert!(response.ok);
        assert_eq!(response.result.unwrap()["waited_fixed_ticks"], 2);
    }

    #[test]
    fn shutdown_command_responds_and_requests_clean_app_exit() {
        use bevy::prelude::*;

        let mut app = App::new();
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);
        let response = control
            .submit(parse_request_line(br#"{"id":24,"method":"process.shutdown"}"#).unwrap())
            .unwrap();

        app.update();
        let response = response.recv().unwrap();

        assert!(response.ok);
        assert_eq!(app.should_exit(), Some(AppExit::Success));
    }

    #[cfg(unix)]
    #[test]
    fn insecure_existing_socket_parent_is_rejected_without_changing_permissions() {
        use std::{
            os::unix::fs::PermissionsExt,
            sync::atomic::{AtomicU64, Ordering},
        };

        static NEXT_PARENT_TEST: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "nano-swarm-agent-parent-{}-{}",
            std::process::id(),
            NEXT_PARENT_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        let socket_path = directory.join("control.sock");

        let result = prepare_control_socket_parent(&socket_path);
        let mode = std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777;
        std::fs::remove_dir(&directory).unwrap();

        assert!(
            matches!(
                &result,
                Err(AgentControlServerError::UnsafeParent(path)) if path == &directory
            ),
            "unexpected parent validation result: {result:?}"
        );
        assert_eq!(mode, 0o755);
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_is_owner_only_and_removed_with_the_app() {
        use std::{
            os::unix::fs::PermissionsExt,
            sync::atomic::{AtomicU64, Ordering},
        };

        use bevy::prelude::*;

        static NEXT_SOCKET_TEST: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "nano-swarm-agent-control-{}-{}",
            std::process::id(),
            NEXT_SOCKET_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        let socket_path = directory.join("control.sock");
        let config = AgentControlConfig::at_socket_path(socket_path.clone());

        {
            let mut app = App::new();
            app.add_plugins(AgentControlPlugin::bind(config).unwrap());

            let mode = std::fs::metadata(&socket_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        assert!(!socket_path.exists());
        remove_control_socket_test_directory(&directory);
    }

    #[cfg(unix)]
    #[test]
    fn lifecycle_lock_serializes_stale_socket_replacement() {
        use std::{
            os::unix::net::UnixListener,
            sync::atomic::{AtomicU64, Ordering},
        };

        static NEXT_LOCK_TEST: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "nano-swarm-agent-lock-{}-{}",
            std::process::id(),
            NEXT_LOCK_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket_path = directory.join("control.sock");
        let stale = UnixListener::bind(&socket_path).unwrap();
        drop(stale);

        let first =
            AgentControlPlugin::bind(AgentControlConfig::at_socket_path(socket_path.clone()))
                .unwrap();
        let second =
            AgentControlPlugin::bind(AgentControlConfig::at_socket_path(socket_path.clone()));

        assert!(matches!(
            second,
            Err(AgentControlServerError::SocketInUse(path)) if path == socket_path
        ));
        drop(first);
        remove_control_socket_test_directory(&directory);
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_survives_unlinked_socket_without_removing_replacement() {
        use std::{
            os::unix::net::UnixListener,
            sync::atomic::{AtomicU64, Ordering},
        };

        use bevy::prelude::*;

        static NEXT_REPLACEMENT_TEST: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "nano-swarm-agent-replacement-{}-{}",
            std::process::id(),
            NEXT_REPLACEMENT_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        let socket_path = directory.join("control.sock");
        let mut app = App::new();
        app.add_plugins(
            AgentControlPlugin::bind(AgentControlConfig::at_socket_path(socket_path.clone()))
                .unwrap(),
        );

        std::fs::remove_file(&socket_path).unwrap();
        let replacement = UnixListener::bind(&socket_path).unwrap();
        drop(app);

        assert!(socket_path.exists());
        drop(replacement);
        std::fs::remove_file(&socket_path).unwrap();
        remove_control_socket_test_directory(&directory);
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_round_trips_ndjson_with_request_id_and_clock() {
        use std::{
            io::{BufRead, BufReader, Write},
            os::unix::net::UnixStream,
            sync::atomic::{AtomicU64, Ordering},
            thread,
        };

        use bevy::prelude::*;

        static NEXT_ROUND_TRIP: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "nano-swarm-agent-round-trip-{}-{}",
            std::process::id(),
            NEXT_ROUND_TRIP.fetch_add(1, Ordering::Relaxed)
        ));
        let socket_path = directory.join("control.sock");
        let mut app = App::new();
        app.add_plugins(
            AgentControlPlugin::bind(AgentControlConfig::at_socket_path(socket_path.clone()))
                .unwrap(),
        );

        let client = thread::spawn(move || {
            let mut stream = UnixStream::connect(socket_path).unwrap();
            stream
                .write_all(b"{\"id\":42,\"method\":\"session.hello\"}\n")
                .unwrap();
            let mut response = String::new();
            BufReader::new(stream).read_line(&mut response).unwrap();
            response
        });
        for _ in 0..10_000 {
            app.update();
            if client.is_finished() {
                break;
            }
            thread::yield_now();
        }
        assert!(client.is_finished(), "control response did not complete");
        let response: serde_json::Value = serde_json::from_str(&client.join().unwrap()).unwrap();

        assert_eq!(response["id"], 42);
        assert_eq!(response["ok"], true);
        assert!(response["frame"].is_u64());
        assert!(response["fixed_tick"].is_u64());
        assert_eq!(response["result"]["protocol_version"], 1);

        drop(app);
        remove_control_socket_test_directory(&directory);
    }

    #[cfg(unix)]
    #[test]
    fn disconnected_wait_client_does_not_block_the_next_client() {
        use std::{
            io::{BufRead, BufReader, Write},
            os::unix::net::UnixStream,
            sync::atomic::{AtomicU64, Ordering},
            thread,
        };

        use bevy::prelude::*;

        static NEXT_CANCEL_TEST: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "nano-swarm-agent-cancel-{}-{}",
            std::process::id(),
            NEXT_CANCEL_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        let socket_path = directory.join("control.sock");
        let mut app = App::new();
        app.add_plugins(
            AgentControlPlugin::bind(AgentControlConfig::at_socket_path(socket_path.clone()))
                .unwrap(),
        );

        {
            let mut abandoned = UnixStream::connect(&socket_path).unwrap();
            let pipelined_id = "x".repeat(16 * 1024);
            abandoned
                .write_all(
                    format!(
                        "{{\"id\":51,\"method\":\"frame.wait\",\"params\":{{\"fixed_ticks\":10000}}}}\n{{\"id\":\"{pipelined_id}\",\"method\":\"session.hello\"}}\n"
                    )
                    .as_bytes(),
                )
                .unwrap();
        }
        let next_socket = socket_path.clone();
        let next_client = thread::spawn(move || {
            let mut stream = UnixStream::connect(next_socket).unwrap();
            stream
                .write_all(b"{\"id\":52,\"method\":\"session.hello\"}\n")
                .unwrap();
            let mut response = String::new();
            BufReader::new(stream).read_line(&mut response).unwrap();
            response
        });

        for _ in 0..20_000 {
            app.update();
            if next_client.is_finished() {
                break;
            }
            thread::yield_now();
        }
        assert!(next_client.is_finished(), "next client remained blocked");
        let response: serde_json::Value =
            serde_json::from_str(&next_client.join().unwrap()).unwrap();
        assert_eq!(response["id"], 52);
        assert_eq!(response["ok"], true);
        for _ in 0..3 {
            app.update();
        }
        assert!(app.world().resource::<PendingWaits>().0.is_empty());

        drop(app);
        remove_control_socket_test_directory(&directory);
    }

    #[cfg(unix)]
    #[test]
    fn request_line_reader_enforces_an_absolute_deadline() {
        use std::{io::Read, time::Duration};

        struct TimedOutReader;

        impl Read for TimedOutReader {
            fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::TimedOut.into())
            }
        }

        impl BufRead for TimedOutReader {
            fn fill_buf(&mut self) -> io::Result<&[u8]> {
                Err(io::ErrorKind::TimedOut.into())
            }

            fn consume(&mut self, _amount: usize) {}
        }

        let mut reader = TimedOutReader;
        let stop = AtomicBool::new(false);
        let start = Instant::now();
        let deadline = start + Duration::from_secs(1);
        let mut clock_reads = 0;

        let error = match read_control_line(&mut reader, &stop, deadline, || {
            clock_reads += 1;
            if clock_reads == 1 {
                start
            } else {
                deadline + Duration::from_secs(1)
            }
        }) {
            Ok(_) => panic!("idle line unexpectedly completed"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[cfg(unix)]
    #[test]
    fn disconnect_probe_distinguishes_buffered_half_close_from_full_close() {
        use std::{io::Write, net::Shutdown, os::unix::net::UnixStream};

        let (half_closed_server, mut half_closed_client) = UnixStream::pair().unwrap();
        half_closed_client.write_all(b"pipelined request").unwrap();
        half_closed_client.shutdown(Shutdown::Write).unwrap();
        assert!(!control_peer_disconnected(&half_closed_server));

        let (closed_server, mut closed_client) = UnixStream::pair().unwrap();
        closed_client.write_all(b"pipelined request").unwrap();
        drop(closed_client);
        assert!(control_peer_disconnected(&closed_server));
    }

    #[cfg(unix)]
    #[test]
    fn write_half_closed_wait_client_receives_its_response() {
        use std::{
            io::{BufRead, BufReader, Write},
            net::Shutdown,
            os::unix::net::UnixStream,
            sync::{
                atomic::{AtomicU64, Ordering},
                mpsc::sync_channel,
            },
            thread,
            time::Duration,
        };

        use bevy::prelude::*;

        static NEXT_HALF_CLOSE_TEST: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "nano-swarm-agent-half-close-{}-{}",
            std::process::id(),
            NEXT_HALF_CLOSE_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        let socket_path = directory.join("control.sock");
        let mut app = App::new();
        app.add_plugins(
            AgentControlPlugin::bind(AgentControlConfig::at_socket_path(socket_path.clone()))
                .unwrap(),
        );

        let (request_ready_tx, request_ready_rx) = sync_channel(0);
        let client = thread::spawn(move || {
            let mut stream = UnixStream::connect(socket_path).unwrap();
            stream
                .write_all(b"{\"id\":53,\"method\":\"frame.wait\",\"params\":{\"frames\":2}}\n")
                .unwrap();
            stream.shutdown(Shutdown::Write).unwrap();
            request_ready_tx.send(()).unwrap();
            let mut response = String::new();
            BufReader::new(stream).read_line(&mut response).unwrap();
            response
        });
        request_ready_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("half-closed client did not finish sending its request");

        for _ in 0..20_000 {
            app.update();
            if client.is_finished() {
                break;
            }
            thread::yield_now();
        }
        assert!(client.is_finished(), "half-closed client remained blocked");
        let response_text = client.join().expect("half-closed client thread panicked");
        let response: serde_json::Value = serde_json::from_str(&response_text)
            .expect("half-closed client must receive a JSON response");
        assert_eq!(response["id"], 53);
        assert_eq!(response["ok"], true);

        drop(app);
        remove_control_socket_test_directory(&directory);
    }

    #[test]
    fn screenshot_command_targets_the_main_cameras_offscreen_image() {
        use std::sync::mpsc::TryRecvError;

        use bevy::{camera::RenderTarget, prelude::*, render::view::screenshot::Screenshot};

        let directory = std::env::temp_dir().join(format!(
            "nano-swarm-screenshot-target-{}",
            std::process::id()
        ));
        let mut images = Assets::<Image>::default();
        let image = images.add(Image::default());
        let expected_target = RenderTarget::from(image.clone());
        let mut app = App::new();
        app.insert_resource(images)
            .insert_resource(AgentScreenshotDirectory(directory.clone()));
        app.world_mut().spawn((MainCamera, expected_target.clone()));
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);

        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(25),
                command: AgentCommand::ScreenshotCapture { name: None },
            })
            .unwrap();
        app.update();

        assert!(matches!(response.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(
            app.world_mut()
                .query::<&Screenshot>()
                .iter(app.world())
                .count(),
            0,
            "capture waits until one frame has been submitted",
        );
        app.update();
        let world = app.world_mut();
        let mut screenshots = world.query::<&Screenshot>();
        let screenshot = screenshots.single(world).unwrap();
        assert_eq!(screenshot.0.as_image(), Some(&image));

        drop(app);
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn screenshot_deferred_before_first_frame_can_be_cancelled() {
        use std::sync::mpsc::TryRecvError;

        use bevy::{camera::RenderTarget, prelude::*, render::view::screenshot::Screenshot};

        let mut app = App::new();
        let image = Handle::<Image>::default();
        app.world_mut()
            .spawn((MainCamera, RenderTarget::from(image)));
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);
        let submitted = control
            .submit_cancellable(AgentRequest {
                id: RequestId::Number(28),
                command: AgentCommand::ScreenshotCapture { name: None },
            })
            .unwrap();

        app.update();
        assert_eq!(
            app.world().resource::<DeferredScreenshotRequests>().0.len(),
            1
        );
        submitted.cancellation.cancel();
        app.update();

        assert!(
            app.world()
                .resource::<DeferredScreenshotRequests>()
                .0
                .is_empty()
        );
        assert_eq!(
            app.world_mut()
                .query::<&Screenshot>()
                .iter(app.world())
                .count(),
            0
        );
        assert!(matches!(
            submitted.response.try_recv(),
            Err(TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn screenshot_completion_skips_cancelled_image_conversion_and_file_creation() {
        use bevy::{prelude::*, render::view::screenshot::ScreenshotCaptured};

        let directory = std::env::temp_dir().join(format!(
            "nano-swarm-cancelled-screenshot-{}",
            std::process::id()
        ));
        let path = directory.join("cancelled.png");
        let mut app = App::new();
        let (_control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);
        let entity = app.world_mut().spawn_empty().id();
        let (reply, response) = sync_channel(1);
        let cancellation = RequestCancellation::default();
        cancellation.cancel();
        app.world_mut()
            .resource_mut::<PendingScreenshots>()
            .0
            .insert(
                entity,
                PendingScreenshot {
                    id: RequestId::Number(27),
                    reply,
                    cancellation,
                    path: path.clone(),
                    capture_clock: None,
                },
            );

        app.world_mut().trigger(ScreenshotCaptured {
            entity,
            image: Image::default(),
        });

        assert!(!path.exists());
        assert!(response.recv().is_err());
    }

    #[test]
    fn screenshot_command_targets_the_main_cameras_primary_window() {
        use std::sync::mpsc::TryRecvError;

        use bevy::{
            camera::RenderTarget, prelude::*, render::view::screenshot::Screenshot,
            window::WindowRef,
        };

        let directory = std::env::temp_dir().join(format!(
            "nano-swarm-window-screenshot-target-{}",
            std::process::id()
        ));
        let mut app = App::new();
        app.insert_resource(AgentScreenshotDirectory(directory.clone()));
        app.world_mut()
            .spawn((MainCamera, RenderTarget::Window(WindowRef::Primary)));
        let (control, plugin) = AgentControlCorePlugin::channel(4);
        app.add_plugins(plugin);
        let response = control
            .submit(AgentRequest {
                id: RequestId::Number(26),
                command: AgentCommand::ScreenshotCapture { name: None },
            })
            .unwrap();

        app.update();
        assert!(matches!(response.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(
            app.world_mut()
                .query::<&Screenshot>()
                .iter(app.world())
                .count(),
            0,
            "capture waits until one frame has been submitted",
        );
        app.update();
        let world = app.world_mut();
        let mut screenshots = world.query::<&Screenshot>();
        assert!(matches!(
            screenshots.single(world).unwrap().0,
            RenderTarget::Window(WindowRef::Primary)
        ));

        drop(app);
        std::fs::remove_dir(directory).unwrap();
    }
}
