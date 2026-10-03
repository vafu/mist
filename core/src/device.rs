//! Device identity metadata exchanged after pairing authorization.

/// The physical form of a Mist device.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum DeviceClass {
    /// A mobile phone.
    Phone,
    /// A tablet device.
    Tablet,
    /// A portable computer.
    Laptop,
    /// A stationary personal computer.
    Desktop,
    /// A headless or server-class computer.
    Server,
    /// A device that does not fit a known class.
    Other,
}

/// The operating-system family hosting Mist.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum Platform {
    /// Android.
    Android,
    /// Linux.
    Linux,
    /// iOS.
    Ios,
    /// macOS.
    Macos,
    /// Windows.
    Windows,
    /// Another operating-system family.
    Other,
}

/// User-visible information identifying a device.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct DeviceInfo {
    /// The device name displayed to its paired peers.
    pub display_name: String,
    /// The physical form of the device.
    pub device_class: DeviceClass,
    /// The operating-system family hosting Mist.
    pub platform: Platform,
}
