use std::process::Command;

const TRACKER_SERVICE: &str = "00001124-0000-1000-8000-00805f9b34fb";
const SONY_VENDOR: &str = "v054c";
const AUDIO_SINK_SERVICE: &str = "0000110b-0000-1000-8000-00805f9b34fb";

fn bluetoothctl(args: &[&str]) -> String {
    Command::new("timeout")
        .arg("15")
        .arg("bluetoothctl")
        .args(args)
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

#[derive(Debug, PartialEq)]
pub enum Scan {
    Supported(String),
    Unsupported,
    Nothing,
}

fn classify(devices: &[(String, String)]) -> Scan {
    let supported = devices.iter().find(|(_, info)| info.contains(TRACKER_SERVICE) && info.contains(SONY_VENDOR));
    match supported {
        Some((address, _)) => Scan::Supported(address.clone()),
        None if devices.iter().any(|(_, info)| info.contains(AUDIO_SINK_SERVICE)) => Scan::Unsupported,
        None => Scan::Nothing,
    }
}

pub fn scan() -> Scan {
    let devices: Vec<(String, String)> = bluetoothctl(&["devices", "Connected"])
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(|address| (address.to_string(), bluetoothctl(&["info", address]).to_lowercase()))
        .collect();
    classify(&devices)
}

pub fn find_headphones() -> Option<String> {
    match scan() {
        Scan::Supported(address) => Some(address),
        _ => None,
    }
}

pub fn connect_tracker(address: &str) {
    bluetoothctl(&["connect", address, TRACKER_SERVICE]);
}

/// Reconnects only the head sensor service. The audio connection stays up.
pub fn reconnect_tracker(address: &str) {
    bluetoothctl(&["disconnect", address, TRACKER_SERVICE]);
    connect_tracker(address);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_separates_supported_unsupported_and_other_devices() {
        let tracker = format!("{TRACKER_SERVICE} {AUDIO_SINK_SERVICE} {SONY_VENDOR}");
        let device = |address: &str, info: &str| (address.to_string(), info.to_string());
        let cases = [
            (vec![], Scan::Nothing),
            (vec![device("mouse", "00001812")], Scan::Nothing),
            (vec![device("speaker", AUDIO_SINK_SERVICE)], Scan::Unsupported),
            (vec![device("speaker", AUDIO_SINK_SERVICE), device("earbuds", &tracker)], Scan::Supported("earbuds".to_string())),
        ];
        for (devices, expected) in cases {
            assert_eq!(classify(&devices), expected, "{devices:?}");
        }
    }
}
