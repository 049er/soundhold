use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const AUTOSWITCH: &str = "bluetooth.autoswitch-to-headset-profile";
const MUSIC_PROFILE: &str = "a2dp-sink";
const OUTPUT_TIMEOUT: Duration = Duration::from_secs(5);

fn wpctl(args: &[&str]) -> Result<String> {
    let output = Command::new("wpctl").args(args).output().context("could not run wpctl")?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn autoswitch_enabled() -> Result<bool> {
    Ok(wpctl(&["settings", AUTOSWITCH])?.contains("Value: true"))
}

fn default_sink() -> Result<Option<String>> {
    let output = wpctl(&["inspect", "@DEFAULT_AUDIO_SINK@"])?;
    Ok(output.split_whitespace().nth(1).map(|id| id.trim_end_matches(',').to_string()))
}

fn pipewire_objects() -> Result<Vec<Value>> {
    let output = Command::new("pw-dump").output().context("could not run pw-dump")?;
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn device_suffix(address: &str) -> String {
    address.replace(':', "_")
}

/// The codec name PipeWire uses for a music profile. `None` for any other profile.
fn profile_codec(profile: &Value) -> Option<String> {
    let suffix = profile["name"].as_str()?.strip_prefix(MUSIC_PROFILE)?;
    if let Some(codec) = suffix.strip_prefix('-') {
        return Some(codec.to_string());
    }
    let (_, codec) = profile["description"].as_str()?.split_once("codec ")?;
    Some(codec.trim_end_matches(')').to_lowercase().replace(['-', ' '], "_"))
}

fn choose_profile<'a>(profiles: &'a [Value], codec: Option<&str>) -> Result<&'a Value> {
    let found = match codec {
        Some(wanted) => profiles.iter().find(|p| profile_codec(p).as_deref() == Some(wanted)),
        None => profiles.iter().find(|p| p["name"] == MUSIC_PROFILE),
    };
    found.ok_or_else(|| {
        let offered: Vec<String> = profiles.iter().filter_map(profile_codec).collect();
        anyhow!("codec {} is not offered, available: {}", codec.unwrap_or("default"), offered.join(", "))
    })
}

/// The name of the output node that belongs to the card with id `card`.
fn output_of(objects: &[Value], card: u64, codec: Option<&str>) -> Option<String> {
    objects.iter().find_map(|object| {
        let props = &object["info"]["props"];
        let matches = props["device.id"] == card && props["media.class"] == "Audio/Sink" && codec.is_none_or(|c| props["api.bluez5.codec"] == c);
        matches.then(|| props["node.name"].as_str().map(str::to_string)).flatten()
    })
}

fn wait_for_output(card: u64, codec: Option<&str>) -> Result<String> {
    let started = Instant::now();
    loop {
        if let Some(name) = output_of(&pipewire_objects()?, card, codec) {
            return Ok(name);
        }
        if started.elapsed() >= OUTPUT_TIMEOUT {
            return Err(anyhow!("the Bluetooth output did not appear"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Keeps the headphones in music mode, and puts the settings back when dropped.
pub struct Session {
    address: String,
    requested_codec: Option<String>,
    autoswitch: bool,
    sink: Option<String>,
    restore_profile: Option<(String, String)>,
    pub output: String,
    pub codec: Option<String>,
    pub codecs: Vec<String>,
}

impl Session {
    pub fn begin(address: &str, codec: Option<&str>) -> Result<Self> {
        let mut session = Self {
            address: address.to_string(),
            requested_codec: codec.map(str::to_string),
            autoswitch: autoswitch_enabled()?,
            sink: None,
            restore_profile: None,
            output: String::new(),
            codec: None,
            codecs: Vec::new(),
        };
        session.hold_music_mode()?;
        Ok(session)
    }

    pub fn hold_music_mode(&mut self) -> Result<()> {
        wpctl(&["settings", AUTOSWITCH, "false"])?;
        self.apply_codec()
    }

    pub fn release_music_mode(&self) -> Result<()> {
        wpctl(&["settings", AUTOSWITCH, if self.autoswitch { "true" } else { "false" }])?;
        Ok(())
    }

    pub fn set_codec(&mut self, codec: Option<&str>) -> Result<()> {
        self.requested_codec = codec.map(str::to_string);
        self.apply_codec()
    }

    fn apply_codec(&mut self) -> Result<()> {
        let (card, codec) = self.select_profile()?;
        self.codec = codec;
        self.output = wait_for_output(card, self.codec.as_deref())?;
        Ok(())
    }

    /// Returns the card id and the codec of the chosen profile.
    fn select_profile(&mut self) -> Result<(u64, Option<String>)> {
        let card_name = format!("bluez_card.{}", device_suffix(&self.address));
        let codec = self.requested_codec.as_deref();
        let objects = pipewire_objects()?;
        let card = objects
            .iter()
            .find(|o| o["info"]["props"]["device.name"] == card_name.as_str())
            .ok_or_else(|| anyhow!("the headphones are not available in PipeWire"))?;
        let card_number = card["id"].as_u64().ok_or_else(|| anyhow!("the headphones have no PipeWire id"))?;
        let params = &card["info"]["params"];
        let profiles = params["EnumProfile"].as_array().map_or(&[][..], Vec::as_slice);
        let chosen = choose_profile(profiles, codec)?;
        let chosen_codec = profile_codec(chosen);
        self.codecs = profiles.iter().filter_map(profile_codec).collect();
        let active = &params["Profile"][0]["index"];
        if *active != chosen["index"] {
            let card_id = card["id"].to_string();
            wpctl(&["set-profile", &card_id, &chosen["index"].to_string()])?;
            if codec.is_some() && self.restore_profile.is_none() {
                self.restore_profile = Some((card_id, active.to_string()));
            }
        }
        Ok((card_number, chosen_codec))
    }

    pub fn use_output(&mut self, node: u32) -> Result<()> {
        if self.sink.is_none() {
            self.sink = default_sink()?;
        }
        wpctl(&["set-default", &node.to_string()])?;
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(sink) = &self.sink {
            let _ = wpctl(&["set-default", sink]);
        }
        if let Some((card, profile)) = &self.restore_profile {
            let _ = wpctl(&["set-profile", card, profile]);
        }
        let _ = self.release_music_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn output_of_matches_only_sinks_of_the_card() {
        let node = |id: u64, class: &str, name: &str| json!({"info": {"props": {"device.id": id, "media.class": class, "node.name": name, "api.bluez5.codec": "aac"}}});
        let objects = [node(7, "Audio/Sink", "fake"), node(5, "Audio/Source", "mic"), node(5, "Audio/Sink", "buds")];
        assert_eq!(output_of(&objects, 5, Some("aac")), Some("buds".to_string()));
        assert_eq!(output_of(&objects, 5, Some("sbc")), None);
    }

    #[test]
    fn choose_profile_matches_the_requested_codec() {
        let profiles = [
            json!({"name": "off", "description": "Off"}),
            json!({"name": "a2dp-sink-sbc", "description": "High Fidelity Playback (A2DP Sink, codec SBC)"}),
            json!({"name": "a2dp-sink-sbc_xq", "description": "High Fidelity Playback (A2DP Sink, codec SBC-XQ)"}),
            json!({"name": "a2dp-sink", "description": "High Fidelity Playback (A2DP Sink, codec AAC)"}),
            json!({"name": "headset-head-unit", "description": "Headset Head Unit (HSP/HFP, codec MSBC)"}),
        ];
        let cases = [
            (None, Ok("a2dp-sink")),
            (Some("aac"), Ok("a2dp-sink")),
            (Some("sbc"), Ok("a2dp-sink-sbc")),
            (Some("sbc_xq"), Ok("a2dp-sink-sbc_xq")),
            (Some("msbc"), Err("codec msbc is not offered, available: sbc, sbc_xq, aac".to_string())),
        ];
        for (codec, expected) in cases {
            let found = choose_profile(&profiles, codec).map(|p| p["name"].as_str().unwrap()).map_err(|e| e.to_string());
            assert_eq!(found, expected, "{codec:?}");
        }
    }
}
