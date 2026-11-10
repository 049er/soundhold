use crate::audio::Audio;
use crate::sensor::{self, Quat, SensorState, Shared};
use crate::session::Session;
use anyhow::Result;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::JoinHandle;

const MIN_CHANGE: f64 = 1.0;

pub struct Reading {
    pub state: &'static str,
    pub turn: f64,
    pub height: f64,
}

/// Field order matters: the audio filter has to stop before the session restores the settings.
pub struct Engine {
    address: String,
    audio: Audio,
    session: Session,
    shared: Arc<Shared>,
    reader: Option<JoinHandle<()>>,
    take_output: bool,
    generation: u32,
    spatial: bool,
    sent: Option<Quat>,
    pub updates: u32,
}

impl Engine {
    pub fn start(address: &str, codec: Option<&str>, take_output: bool, sofa: &Path) -> Result<Self> {
        let mut session = Session::begin(address, codec)?;
        let audio = Audio::start(&session.output, sofa, 0)?;
        if take_output {
            session.use_output(audio.node)?;
        }
        let shared = Arc::new(Shared::default());
        let reader = Some(sensor::spawn(shared.clone(), address.to_string()));
        Ok(Self { address: address.to_string(), audio, session, shared, reader, take_output, generation: 0, spatial: true, sent: None, updates: 0 })
    }

    pub fn spatial(&self) -> bool {
        self.spatial
    }

    pub fn codec(&self) -> Option<&str> {
        self.session.codec.as_deref()
    }

    pub fn codecs(&self) -> &[String] {
        &self.session.codecs
    }

    pub fn recentre(&mut self) {
        self.shared.recentre.store(true, Ordering::Relaxed);
        self.sent = None;
    }

    pub fn set_width(&mut self, degrees: f64) {
        self.audio.side_angle = degrees;
        self.sent = None;
    }

    pub fn set_level(&mut self, level: f64) {
        self.audio.wet_gain = level;
        if self.spatial {
            self.audio.set_spatial(true);
        }
    }

    pub fn set_codec(&mut self, codec: Option<&str>) -> Result<()> {
        self.session.set_codec(codec)
    }

    /// Starts a filter with the new head data beside the old one. The old filter keeps playing if this fails.
    pub fn set_head(&mut self, sofa: &Path) -> Result<()> {
        let generation = self.generation + 1;
        let mut audio = Audio::start(&self.session.output, sofa, generation)?;
        audio.side_angle = self.audio.side_angle;
        audio.wet_gain = self.audio.wet_gain;
        audio.set_spatial(self.spatial);
        if self.take_output {
            self.session.use_output(audio.node)?;
        }
        self.generation = generation;
        self.audio = audio;
        self.sent = None;
        Ok(())
    }

    /// When off, the sound passes through unchanged, the sensor is off and the microphone can switch on for calls.
    pub fn set_spatial(&mut self, enabled: bool) -> Result<()> {
        if enabled == self.spatial {
            return Ok(());
        }
        self.spatial = enabled;
        self.sent = None;
        self.audio.set_spatial(enabled);
        if enabled {
            self.session.hold_music_mode()?;
            self.reader = Some(sensor::spawn(self.shared.clone(), self.address.clone()));
        } else {
            self.stop_reader();
            self.session.release_music_mode()?;
        }
        Ok(())
    }

    fn stop_reader(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.shared.stop.store(false, Ordering::Relaxed);
        self.shared.set_state(SensorState::Searching);
        *self.shared.rotation.lock().unwrap() = None;
    }

    /// Sends the latest head position to the sound filter.
    pub fn tick(&mut self) -> Reading {
        let rotation = *self.shared.rotation.lock().unwrap();
        if let (true, Some(value)) = (self.spatial, rotation) {
            if self.sent.is_none_or(|last| sensor::angle_between(last, value) >= MIN_CHANGE) {
                self.audio.set_rotation(value);
                self.sent = Some(value);
                self.updates += 1;
            }
        }
        let state = match (self.spatial, *self.shared.state.lock().unwrap()) {
            (false, _) => "normal",
            (true, SensorState::Searching) => "reconnecting",
            (true, SensorState::NoAccess) => "no_access",
            (true, SensorState::NoData) => "no_data",
            (true, SensorState::Working) => "spatial",
        };
        let (turn, height) = rotation.map_or((0.0, 0.0), |r| sensor::direction(r, 0.0));
        Reading { state, turn, height }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop_reader();
    }
}
