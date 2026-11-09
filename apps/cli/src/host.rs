//! Linux host identity used when describing this device to peers.

use std::env;
use std::fs;

use mist_app_contract::{DeviceClass, DeviceInfo, Platform};

const KERNEL_HOSTNAME_PATH: &str = "/proc/sys/kernel/hostname";
const FALLBACK_DISPLAY_NAME: &str = "Mist Linux";

/// Describes the local Linux installation to paired devices.
///
/// The kernel hostname is authoritative because service managers do not
/// reliably export `HOSTNAME`.
#[must_use]
pub fn local_device_info() -> DeviceInfo {
    DeviceInfo {
        display_name: local_display_name().unwrap_or_else(|| FALLBACK_DISPLAY_NAME.to_owned()),
        device_class: DeviceClass::Desktop,
        platform: Platform::Linux,
    }
}

fn local_display_name() -> Option<String> {
    display_name_from([
        fs::read_to_string(KERNEL_HOSTNAME_PATH).ok(),
        env::var("HOSTNAME").ok(),
    ])
}

fn display_name_from(candidates: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    candidates.into_iter().flatten().find_map(|candidate| {
        let name = candidate.trim();
        (!name.is_empty()).then(|| name.to_owned())
    })
}

#[cfg(test)]
mod test {
    use super::{display_name_from, local_display_name};

    #[test]
    fn display_name_prefers_the_first_non_empty_candidate() {
        assert_eq!(
            display_name_from([
                None,
                Some("  L47W0D74\n".to_owned()),
                Some("fallback".to_owned())
            ]),
            Some("L47W0D74".to_owned())
        );
        assert_eq!(display_name_from([Some(" ".to_owned()), None]), None);
    }

    #[test]
    fn local_display_name_is_available_on_linux() {
        assert!(local_display_name().is_some());
    }
}
