//! Shared Linux D-Bus contract used by the daemon and CLI.

pub mod clipboard;

/// Well-known session bus name owned by `mistd`.
pub const BUS_NAME: &str = "io.github.vafu.Mist";
/// Root object exported by `mistd`.
pub const OBJECT_PATH: &str = "/io/github/vafu/Mist";
/// Versioned Mist D-Bus interface.
pub const INTERFACE_NAME: &str = "io.github.vafu.Mist1";

/// Serialized D-Bus device row: endpoint ID, display name, device class,
/// platform, status, transport, selected.
pub type DeviceRow = (String, String, String, String, String, String, bool);

#[zbus::proxy(
    interface = "io.github.vafu.Mist1",
    default_service = "io.github.vafu.Mist",
    default_path = "/io/github/vafu/Mist"
)]
pub trait Mist {
    /// Local endpoint identity.
    #[zbus(property)]
    fn endpoint_id(&self) -> zbus::Result<String>;

    /// Current paired-device snapshots.
    #[zbus(property)]
    fn devices(&self) -> zbus::Result<Vec<DeviceRow>>;

    /// Whether at least one device is connected.
    #[zbus(property)]
    fn connected(&self) -> zbus::Result<bool>;

    /// Creates a pairing offer and starts accepting it.
    fn start_pairing(&self) -> zbus::Result<String>;

    /// Cancels the active pairing offer.
    fn cancel_pairing(&self) -> zbus::Result<()>;

    /// Selects or deselects a device for default feature sends.
    fn select_device(&self, endpoint_id: &str, selected: bool) -> zbus::Result<()>;

    /// Connects a known paired device.
    fn connect_device(&self, endpoint_id: &str) -> zbus::Result<()>;

    /// Disconnects one device.
    fn disconnect_device(&self, endpoint_id: &str) -> zbus::Result<()>;

    /// Revokes trust and removes one device.
    fn forget_device(&self, endpoint_id: &str) -> zbus::Result<()>;

    /// Sends a notification to selected devices.
    fn push_notification(&self, title: &str, body: &str) -> zbus::Result<()>;

    /// Sends text clipboard content to selected devices.
    fn push_clipboard_text(&self, text: &str) -> zbus::Result<()>;

    /// Sends a local file as clipboard content to selected devices.
    fn push_clipboard_blob(&self, path: &str, mime_type: &str) -> zbus::Result<()>;

    /// Requests an orderly daemon shutdown.
    fn shutdown(&self) -> zbus::Result<()>;

    /// A paired-device snapshot changed.
    #[zbus(signal)]
    fn device_changed(&self, endpoint_id: &str) -> zbus::Result<()>;

    /// A notification arrived from another device.
    #[zbus(signal)]
    fn notification_received(
        &self,
        endpoint_id: &str,
        application: &str,
        title: &str,
        body: &str,
    ) -> zbus::Result<()>;

    /// Background pairing failed after an offer was returned.
    #[zbus(signal)]
    fn pairing_failed(&self, reason: &str) -> zbus::Result<()>;

    /// Clipboard content arrived from another device.
    #[zbus(signal)]
    fn clipboard_received(
        &self,
        endpoint_id: &str,
        mime_type: &str,
        text: &str,
        path: &str,
    ) -> zbus::Result<()>;
}
