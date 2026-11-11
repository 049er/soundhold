use std::process::Command;

const TRACKER_SERVICE: &str = "00001124-0000-1000-8000-00805f9b34fb";
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
    Supported { address: String, description: String },
    Unsupported { description: String },
    Nothing,
}

fn offers(info: &str, service: &str) -> bool {
    info.to_lowercase().contains(service)
}

/// Model details for a report. Leaves out the address and the name, which can hold the owner's name.
fn describe(info: &str) -> String {
    let ids = info.lines().find_map(|line| line.trim().strip_prefix("Modalias:")).map_or("unknown", str::trim);
    let tracker = if offers(info, TRACKER_SERVICE) { "yes" } else { "no" };
    format!("Bluetooth IDs: {ids}\nHID service: {tracker}\n")
}

fn classify(devices: &[(String, String)]) -> Scan {
    let audio = |info: &str| offers(info, AUDIO_SINK_SERVICE);
    if let Some((address, info)) = devices.iter().find(|(_, info)| audio(info) && offers(info, TRACKER_SERVICE)) {
        return Scan::Supported { address: address.clone(), description: describe(info) };
    }
    match devices.iter().find(|(_, info)| audio(info)) {
        Some((_, info)) => Scan::Unsupported { description: describe(info) },
        None => Scan::Nothing,
    }
}

pub fn scan() -> Scan {
    let devices: Vec<(String, String)> = bluetoothctl(&["devices", "Connected"])
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(|address| (address.to_string(), bluetoothctl(&["info", address])))
        .collect();
    classify(&devices)
}

pub fn find_headphones() -> Option<String> {
    match scan() {
        Scan::Supported { address, .. } => Some(address),
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
        let device = |address: &str, info: &str| (address.to_string(), info.to_string());
        let full = format!("Name: Buds\nModalias: usb:v1234\n{TRACKER_SERVICE}\n{AUDIO_SINK_SERVICE}\n");
        let speaker = format!("Name: Speaker\n{AUDIO_SINK_SERVICE}\n");
        let cases = [
            (vec![], Scan::Nothing),
            (vec![device("mouse", TRACKER_SERVICE)], Scan::Nothing),
            (
                vec![device("speaker", &speaker)],
                Scan::Unsupported { description: "Bluetooth IDs: unknown\nHID service: no\n".to_string() },
            ),
            (
                vec![device("speaker", &speaker), device("buds", &full)],
                Scan::Supported { address: "buds".to_string(), description: "Bluetooth IDs: usb:v1234\nHID service: yes\n".to_string() },
            ),
        ];
        for (devices, expected) in cases {
            assert_eq!(classify(&devices), expected, "{devices:?}");
        }
    }
}
