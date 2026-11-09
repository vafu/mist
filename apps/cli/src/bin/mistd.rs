use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use mist_app_contract::{
    ClipboardContent, ConnectionTransport, DeviceClass, DeviceInfo, DeviceStatus, ManagedDevice,
    MediaCommand, MistApp, Notification, NotificationAction, NotificationImage, NotificationKind,
    Platform,
};
use mist_core::host::MistFactory;
use mist_linux::clipboard::{LocalClipboardProvider, WaylandClipboard};
use mist_linux::media::{MprisMedia, MprisPublisher};
use mist_linux::{BUS_NAME, DeviceRow, MediaRow, OBJECT_PATH};
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedValue;
use zbus::{Connection, MessageStream};

static NOTIFICATION_ID: AtomicU64 = AtomicU64::new(1);

type NotifyBody = (
    String,
    u32,
    String,
    String,
    String,
    Vec<String>,
    HashMap<String, OwnedValue>,
    i32,
);

#[derive(Debug)]
struct MistDbus {
    runtime: Arc<dyn MistApp>,
    connection: Connection,
    pairing_task: Mutex<Option<JoinHandle<()>>>,
    shutdown: watch::Sender<bool>,
}

impl MistDbus {
    fn new(
        runtime: Arc<dyn MistApp>,
        connection: Connection,
        shutdown: watch::Sender<bool>,
    ) -> Self {
        Self {
            runtime,
            connection,
            pairing_task: Mutex::new(None),
            shutdown,
        }
    }
}

#[zbus::interface(name = "io.github.vafu.Mist1")]
impl MistDbus {
    #[zbus(property(emits_changed_signal = "const"), name = "EndpointId")]
    fn endpoint_id(&self) -> String {
        self.runtime.endpoint_id()
    }

    #[zbus(property, name = "Devices")]
    fn devices(&self) -> Vec<DeviceRow> {
        self.runtime
            .device_manager()
            .list()
            .iter()
            .map(device_row)
            .collect()
    }

    #[zbus(property, name = "MediaPlayers")]
    fn media_players(&self) -> Vec<MediaRow> {
        self.runtime
            .media()
            .list()
            .into_iter()
            .map(media_row)
            .collect()
    }

    async fn media_command(&self, player_id: &str, command: &str) -> zbus::fdo::Result<()> {
        let command = parse_media_command(command).ok_or_else(|| {
            zbus::fdo::Error::InvalidArgs(format!("unknown media command {command}"))
        })?;
        self.runtime
            .media()
            .command(player_id.to_owned(), command)
            .await
            .map_err(fdo_error)
    }

    #[zbus(property, name = "Connected")]
    fn connected(&self) -> bool {
        self.runtime
            .device_manager()
            .list()
            .iter()
            .any(|device| device.status == DeviceStatus::Connected)
    }

    async fn start_pairing(&self) -> zbus::fdo::Result<String> {
        let mut task = self.pairing_task.lock().await;
        if task.as_ref().is_some_and(|task| !task.is_finished()) {
            return Err(zbus::fdo::Error::Failed(
                "a pairing offer is already active".to_owned(),
            ));
        }

        let pairing = self.runtime.pairing();
        let offer = pairing.initiate().map_err(fdo_error)?;
        let emitter = SignalEmitter::new(&self.connection, OBJECT_PATH)
            .map_err(fdo_error)?
            .to_owned();
        *task = Some(tokio::spawn(async move {
            if let Err(error) = pairing.wait_for_peer().await {
                let _ = emitter.pairing_failed(&error.to_string()).await;
            }
        }));
        Ok(offer.encoded)
    }

    fn cancel_pairing(&self) {
        self.runtime.pairing().cancel();
    }

    fn select_device(&self, endpoint_id: &str, selected: bool) -> zbus::fdo::Result<()> {
        self.runtime
            .device_manager()
            .select(endpoint_id.to_owned(), selected)
            .map_err(fdo_error)
    }

    async fn connect_device(&self, endpoint_id: &str) -> zbus::fdo::Result<()> {
        self.runtime
            .device_manager()
            .connect(endpoint_id.to_owned())
            .await
            .map_err(fdo_error)
    }

    fn disconnect_device(&self, endpoint_id: &str) -> zbus::fdo::Result<()> {
        self.runtime
            .device_manager()
            .disconnect(endpoint_id.to_owned())
            .map_err(fdo_error)
    }

    fn forget_device(&self, endpoint_id: &str) -> zbus::fdo::Result<()> {
        self.runtime
            .device_manager()
            .forget(endpoint_id.to_owned())
            .map_err(fdo_error)
    }

    async fn push_clipboard_text(&self, text: &str) -> zbus::fdo::Result<()> {
        self.runtime
            .clipboard()
            .push(
                ClipboardContent::Text {
                    text: text.to_owned(),
                },
                Vec::new(),
            )
            .await
            .map_err(fdo_error)
    }

    async fn push_clipboard_blob(&self, path: &str, mime_type: &str) -> zbus::fdo::Result<()> {
        self.runtime
            .clipboard()
            .push(
                ClipboardContent::Blob {
                    mime_type: mime_type.to_owned(),
                    path: path.to_owned(),
                },
                Vec::new(),
            )
            .await
            .map_err(fdo_error)
    }

    async fn push_notification(
        &self,
        title: &str,
        body: &str,
        image_path: &str,
    ) -> zbus::fdo::Result<()> {
        let image = (!image_path.is_empty()).then(|| NotificationImage {
            mime_type: guess_image_mime(image_path),
            path: image_path.to_owned(),
        });
        self.runtime
            .notification_channel()
            .push(
                Notification {
                    id: format!("linux:{}", NOTIFICATION_ID.fetch_add(1, Ordering::Relaxed)),
                    application: "Mist CLI".to_owned(),
                    title: title.to_owned(),
                    body: body.to_owned(),
                    timestamp_ms: unix_millis(),
                    silent: false,
                    kind: NotificationKind::Message,
                    app_icon: None,
                    image,
                    actions: Vec::new(),
                },
                Vec::new(),
            )
            .await
            .map_err(fdo_error)
    }

    fn shutdown(&self) {
        self.shutdown.send_replace(true);
    }

    #[zbus(signal)]
    async fn device_changed(
        signal_emitter: &SignalEmitter<'_>,
        endpoint_id: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn notification_received(
        signal_emitter: &SignalEmitter<'_>,
        endpoint_id: &str,
        application: &str,
        title: &str,
        body: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn pairing_failed(signal_emitter: &SignalEmitter<'_>, reason: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn clipboard_received(
        signal_emitter: &SignalEmitter<'_>,
        endpoint_id: &str,
        mime_type: &str,
        text: &str,
        path: &str,
    ) -> zbus::Result<()>;
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("mistd: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let mpris = MprisMedia::new().await.ok();
    let mut factory = MistFactory::new(state_directory()?, local_device_info())?;
    if let Some(mpris) = mpris.as_ref() {
        factory = factory.media(mpris.clone());
    }
    let runtime = factory.create().await?;
    let connection = Connection::session().await?;
    connection.request_name(BUS_NAME).await?;
    let (shutdown, mut shutdown_rx) = watch::channel(false);
    connection
        .object_server()
        .at(
            OBJECT_PATH,
            MistDbus::new(Arc::clone(&runtime), connection.clone(), shutdown),
        )
        .await?;

    let mut device_events = tokio::spawn(forward_device_events(
        connection.clone(),
        Arc::clone(&runtime),
    ));
    let action_origins: ActionOrigins = Arc::new(Mutex::new(HashMap::new()));
    let mut incoming_notifications = tokio::spawn(forward_incoming_notifications(
        connection.clone(),
        Arc::clone(&runtime),
        Arc::clone(&action_origins),
    ));
    let mut monitored_notifications = tokio::spawn(monitor_desktop_notifications(
        connection.clone(),
        Arc::clone(&runtime),
        Arc::clone(&action_origins),
    ));
    let local_clipboard: Arc<dyn LocalClipboardProvider> =
        Arc::new(WaylandClipboard::new(state_directory()?.join("clipboard")));
    let mut incoming_clipboard = tokio::spawn(forward_incoming_clipboard(
        connection.clone(),
        Arc::clone(&runtime),
        Arc::clone(&local_clipboard),
    ));
    let mut local_clipboard_changes =
        tokio::spawn(watch_local_clipboard(Arc::clone(&runtime), local_clipboard));
    let mut published_media = tokio::spawn(publish_remote_media(
        connection.clone(),
        Arc::clone(&runtime),
    ));

    println!("mistd ready on {BUS_NAME} ({})", runtime.endpoint_id());
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        changed = shutdown_rx.changed() => {
            changed?;
        }
        result = &mut device_events => result??,
        result = &mut incoming_notifications => result??,
        result = &mut monitored_notifications => result??,
        result = &mut incoming_clipboard => result??,
        result = &mut local_clipboard_changes => result??,
        result = &mut published_media => result??,
    }

    device_events.abort();
    incoming_notifications.abort();
    monitored_notifications.abort();
    incoming_clipboard.abort();
    local_clipboard_changes.abort();
    published_media.abort();
    runtime.shutdown().await;
    Ok(())
}

async fn forward_device_events(
    connection: Connection,
    runtime: Arc<dyn MistApp>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let events = runtime.device_manager().subscribe();
    loop {
        let event = events.next().await?;
        let interface = connection
            .object_server()
            .interface::<_, MistDbus>(OBJECT_PATH)
            .await?;
        interface.device_changed(&event.device.endpoint_id).await?;
        let emitter = interface.signal_emitter();
        let api = interface.get().await;
        api.devices_changed(emitter).await?;
        api.connected_changed(emitter).await?;
    }
}

/// Maps a desktop notification id back to the device and notification that
/// produced it, so `ActionInvoked` can be routed home.
type ActionOrigins = Arc<Mutex<HashMap<u32, (String, String, Vec<String>)>>>;

/// How many desktop notifications keep their action mapping.
const MAX_TRACKED_DESKTOP_NOTIFICATIONS: usize = 256;

async fn forward_incoming_notifications(
    connection: Connection,
    runtime: Arc<dyn MistApp>,
    origins: ActionOrigins,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let notifications = runtime.notification_channel().subscribe();
    loop {
        let received = notifications.next().await?;
        let desktop_id = show_desktop_notification(
            &connection,
            &received.source_device.display_name,
            &received.notification,
        )
        .await?;
        if !received.notification.actions.is_empty() {
            let mut origins = origins.lock().await;
            if origins.len() >= MAX_TRACKED_DESKTOP_NOTIFICATIONS {
                origins.clear();
            }
            origins.insert(
                desktop_id,
                (
                    received.source_endpoint_id.clone(),
                    received.notification.id.clone(),
                    received
                        .notification
                        .actions
                        .iter()
                        .map(|action| action.id.clone())
                        .collect(),
                ),
            );
        }
        let interface = connection
            .object_server()
            .interface::<_, MistDbus>(OBJECT_PATH)
            .await?;
        interface
            .notification_received(
                &received.source_endpoint_id,
                &received.notification.application,
                &received.notification.title,
                &received.notification.body,
            )
            .await?;
    }
}

/// Resolves an `image-path` style hint (URI or file path) into a local image.
fn hint_image(hints: &HashMap<String, OwnedValue>, key: &str) -> Option<NotificationImage> {
    let value = hints.get(key)?;
    let raw = <&str>::try_from(value).ok()?;
    let path = raw.strip_prefix("file://").unwrap_or(raw);
    let path = PathBuf::from(percent_decode(path));
    attach_image(&path, key.contains("icon"))
}

fn file_icon(app_icon: &str) -> Option<NotificationImage> {
    if app_icon.is_empty() {
        return None;
    }
    let path = app_icon.strip_prefix("file://").unwrap_or(app_icon);
    attach_image(&PathBuf::from(percent_decode(path)), true)
}

fn attach_image(path: &std::path::Path, small: bool) -> Option<NotificationImage> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let mime_type = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        _ => return None,
    };
    if !path.is_file() {
        return None;
    }
    let _ = small;
    Some(NotificationImage {
        mime_type: mime_type.to_owned(),
        path: path.to_string_lossy().into_owned(),
    })
}

/// Parses freedesktop action pairs into actions.
///
/// The wire format is a flat `[id, label, id, label, ...]` list. The synthetic
/// `default` action is dropped because it represents a body click rather than a
/// button.
fn parse_notify_actions(raw: &[String]) -> Vec<NotificationAction> {
    raw.chunks_exact(2)
        .filter(|pair| pair[0] != "default")
        .map(|pair| NotificationAction {
            id: pair[0].clone(),
            label: pair[1].clone(),
        })
        .collect()
}

fn guess_image_mime(path: &str) -> String {
    match std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg".to_owned(),
        Some("webp") => "image/webp".to_owned(),
        Some("gif") => "image/gif".to_owned(),
        _ => "image/png".to_owned(),
    }
}

fn percent_decode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or_default();
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte as char);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index] as char);
        index += 1;
    }
    out
}

/// Routes a desktop `ActionInvoked` signal back to the owning device.
async fn handle_action_invoked(
    runtime: &Arc<dyn MistApp>,
    action_origins: &ActionOrigins,
    message: &zbus::Message,
) {
    let Ok((desktop_id, key)) = message.body().deserialize::<(u32, String)>() else {
        return;
    };
    let Some((endpoint_id, notification_id, actions)) =
        action_origins.lock().await.get(&desktop_id).cloned()
    else {
        return;
    };
    if !actions.contains(&key) {
        return;
    }
    if let Err(error) = runtime
        .notification_channel()
        .activate(endpoint_id, notification_id, key)
        .await
    {
        eprintln!("mistd: could not deliver notification action: {error}");
    }
}

async fn show_desktop_notification(
    connection: &Connection,
    device_name: &str,
    notification: &Notification,
) -> zbus::Result<u32> {
    let mut actions: Vec<&str> = Vec::with_capacity(notification.actions.len() * 2);
    for action in &notification.actions {
        actions.push(&action.id);
        actions.push(&action.label);
    }
    if notification.actions.is_empty() {
        actions.push("default");
        actions.push("Open");
    }
    let mut hints: HashMap<&str, OwnedValue> = HashMap::new();
    for path in [
        notification.app_icon.as_ref().map(|icon| &icon.path),
        notification.image.as_ref().map(|image| &image.path),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(value) =
            OwnedValue::try_from(zbus::zvariant::Value::from(format!("file://{path}")))
        {
            hints.insert("image-path", value);
        }
    }
    let replaces_id = 0_u32;
    connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                format!("{} · {device_name}", notification.application),
                replaces_id,
                "dialog-information",
                &notification.title,
                &notification.body,
                actions,
                hints,
                -1_i32,
            ),
        )
        .await?
        .body()
        .deserialize::<u32>()
}

/// Watches desktop notifications and their action activations.
///
/// D-Bus allows one monitor per connection, so a single connection carries both
/// rules and dispatches on the member name.
async fn monitor_desktop_notifications(
    daemon_connection: Connection,
    runtime: Arc<dyn MistApp>,
    action_origins: ActionOrigins,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let monitor = Connection::session().await?;
    let rules = [
        "type='method_call',path='/org/freedesktop/Notifications',interface='org.freedesktop.Notifications',member='Notify'",
        "type='signal',interface='org.freedesktop.Notifications',member='ActionInvoked'",
    ];
    monitor
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus.Monitoring"),
            "BecomeMonitor",
            &(&rules[..], 0_u32),
        )
        .await?;

    let daemon_name = daemon_connection
        .unique_name()
        .map(ToString::to_string)
        .unwrap_or_default();
    let mut messages = MessageStream::from(monitor);
    while let Some(message) = messages.next().await {
        let message = message?;
        if message
            .header()
            .sender()
            .is_some_and(|sender| sender.as_str() == daemon_name)
        {
            continue;
        }
        if message
            .header()
            .member()
            .is_some_and(|member| member.as_str() == "ActionInvoked")
        {
            handle_action_invoked(&runtime, &action_origins, &message).await;
            continue;
        }
        let Ok((application, _, app_icon, title, body, raw_actions, hints, _)) =
            message.body().deserialize::<NotifyBody>()
        else {
            continue;
        };
        let icon = hint_image(&hints, "image-path")
            .or_else(|| hint_image(&hints, "image_path"))
            .or_else(|| hint_image(&hints, "app_icon"))
            .or_else(|| file_icon(&app_icon));
        let image = hint_image(&hints, "image-data").or_else(|| hint_image(&hints, "image_data"));
        let notification = Notification {
            id: format!("linux:{}", NOTIFICATION_ID.fetch_add(1, Ordering::Relaxed)),
            application,
            title,
            body,
            timestamp_ms: unix_millis(),
            silent: false,
            kind: NotificationKind::Message,
            app_icon: icon,
            image,
            actions: parse_notify_actions(&raw_actions),
        };
        if let Err(error) = runtime
            .notification_channel()
            .push(notification, Vec::new())
            .await
        {
            eprintln!("mistd: could not forward desktop notification: {error}");
        }
    }
    Ok(())
}

/// Watches the native clipboard and schedules a send for each local change.
///
/// The watcher blocks on Wayland, so it runs on a blocking thread and forwards
/// content over a channel. Echoes are filtered by core using send origin.
async fn watch_local_clipboard(
    runtime: Arc<dyn MistApp>,
    provider: Arc<dyn LocalClipboardProvider>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (changes, mut receiver) = tokio::sync::mpsc::channel(16);
    std::thread::Builder::new()
        .name("mist-clipboard-watch".to_owned())
        .spawn(move || {
            let Ok(mut watcher) = provider.watch() else {
                eprintln!("mistd: local clipboard watch unavailable");
                return;
            };
            loop {
                match watcher.next() {
                    Ok(Some(content)) => {
                        if changes.blocking_send(content).is_err() {
                            return;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("mistd: local clipboard watch stopped: {error}");
                        return;
                    }
                }
            }
        })?;

    while let Some(content) = receiver.recv().await {
        if let Err(error) = runtime.clipboard().push(content, Vec::new()).await {
            eprintln!("mistd: could not forward local clipboard: {error}");
        }
    }
    Ok(())
}

async fn forward_incoming_clipboard(
    connection: Connection,
    runtime: Arc<dyn MistApp>,
    local_clipboard: Arc<dyn LocalClipboardProvider>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let clipboard = runtime.clipboard().subscribe();
    loop {
        let received = clipboard.next().await?;
        if let Err(error) = local_clipboard.apply(&received.content) {
            eprintln!("mistd: could not apply remote clipboard: {error}");
        }
        let (mime_type, text, path) = match &received.content {
            ClipboardContent::Text { text } => (
                "text/plain;charset=utf-8".to_owned(),
                text.clone(),
                String::new(),
            ),
            ClipboardContent::Blob { mime_type, path } => {
                (mime_type.clone(), String::new(), path.clone())
            }
        };
        let interface = connection
            .object_server()
            .interface::<_, MistDbus>(OBJECT_PATH)
            .await?;
        interface
            .clipboard_received(&received.source_endpoint_id, &mime_type, &text, &path)
            .await?;
    }
}

/// Exports remote players as local MPRIS players for desktop media controls.
async fn publish_remote_media(
    connection: Connection,
    runtime: Arc<dyn MistApp>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let publisher = Arc::new(MprisPublisher::new(connection, runtime.media()));
    publisher.run().await;
    Ok(())
}

fn parse_media_command(command: &str) -> Option<MediaCommand> {
    Some(match command {
        "play" => MediaCommand::Play,
        "pause" => MediaCommand::Pause,
        "play-pause" => MediaCommand::PlayPause,
        "stop" => MediaCommand::Stop,
        "next" => MediaCommand::Next,
        "previous" => MediaCommand::Previous,
        _ => return None,
    })
}

fn media_row(player: mist_app_contract::MediaPlayer) -> MediaRow {
    (
        player.id,
        player.endpoint_id,
        player.name,
        player.device.display_name,
        format!("{:?}", player.origin).to_lowercase(),
        format!("{:?}", player.status).to_lowercase(),
        player.metadata.title,
        player.metadata.artist,
        player.metadata.album,
    )
}

fn device_row(device: &ManagedDevice) -> DeviceRow {
    (
        device.endpoint_id.clone(),
        device.device.display_name.clone(),
        format!("{:?}", device.device.device_class).to_lowercase(),
        format!("{:?}", device.device.platform).to_lowercase(),
        match device.status {
            DeviceStatus::Disconnected => "disconnected",
            DeviceStatus::Connecting => "connecting",
            DeviceStatus::Connected => "connected",
        }
        .to_owned(),
        match device.transport {
            ConnectionTransport::Direct => "direct",
            ConnectionTransport::Relay => "relay",
            ConnectionTransport::Custom => "custom",
            ConnectionTransport::Unknown => "unknown",
        }
        .to_owned(),
        device.selected,
    )
}

fn local_device_info() -> DeviceInfo {
    let display_name = env::var("HOSTNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Mist Linux".to_owned());
    DeviceInfo {
        display_name,
        device_class: DeviceClass::Desktop,
        platform: Platform::Linux,
    }
}

fn state_directory() -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
    if let Some(path) = env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(path).join("mist"));
    }
    let home = env::var_os("HOME").ok_or("HOME is not set and XDG_STATE_HOME is unavailable")?;
    Ok(PathBuf::from(home).join(".local/state/mist"))
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

fn fdo_error(error: impl std::fmt::Display) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(error.to_string())
}
