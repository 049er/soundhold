use crate::sensor::{self, Quat};
use crate::settings::{DEFAULT_LEVEL, DEFAULT_WIDTH};
use anyhow::{anyhow, Context, Result};
use pipewire as pw;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Property, PropertyFlags, Value};
use pw::types::ObjectType;
use std::cell::RefCell;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

const FILTER: &str = include_str!("filter.conf");
const DRY_GAIN: f32 = 1.0;

enum Message {
    Params(Vec<(String, f32)>),
    Quit,
}

pub struct Audio {
    sender: pw::channel::Sender<Message>,
    thread: Option<JoinHandle<()>>,
    filter: Child,
    pub node: u32,
    pub side_angle: f64,
    pub wet_gain: f64,
}

/// Escapes text for a quoted string in a PipeWire config file.
fn quoted(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn start_filter(output: &str, sofa: &Path, generation: u32) -> Result<Child> {
    let text = FILTER
        .replace("@SOFA@", &quoted(&sofa.canonicalize()?.to_string_lossy()))
        .replace("@TARGET@", &quoted(output))
        .replace("@GEN@", &generation.to_string());
    let directory = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| anyhow!("XDG_RUNTIME_DIR is not set"))?;
    let config = PathBuf::from(directory).join("soundhold-filter.conf");
    std::fs::write(&config, text)?;
    Command::new("pipewire")
        .arg("-c")
        .arg(&config)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("could not start the PipeWire filter")
}

fn send_params(node: &pw::node::Node, params: &[(String, f32)]) -> Result<()> {
    let mut values = Vec::with_capacity(params.len() * 2);
    for (name, value) in params {
        values.push(Value::String(name.clone()));
        values.push(Value::Float(*value));
    }
    let object = Value::Object(Object {
        type_: pw::spa::sys::SPA_TYPE_OBJECT_Props,
        id: pw::spa::sys::SPA_PARAM_Props,
        properties: vec![Property {
            key: pw::spa::sys::SPA_PROP_params,
            flags: PropertyFlags::empty(),
            value: Value::Struct(values),
        }],
    });
    let bytes = PodSerializer::serialize(Cursor::new(Vec::new()), &object)
        .map_err(|e| anyhow!("{e:?}"))?
        .0
        .into_inner();
    let pod = Pod::from_bytes(&bytes).ok_or_else(|| anyhow!("invalid parameters"))?;
    node.set_param(pw::spa::param::ParamType::Props, 0, pod);
    Ok(())
}

fn run_loop(receiver: pw::channel::Receiver<Message>, ready: mpsc::Sender<u32>, node_name: String) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;
    let node: Rc<RefCell<Option<pw::node::Node>>> = Rc::new(RefCell::new(None));

    let bind_registry = registry.downgrade();
    let bind_node = node.clone();
    let _listener = registry
        .add_listener_local()
        .global(move |global| {
            let name = global.props.as_ref().and_then(|p| p.get("node.name"));
            if global.type_ != ObjectType::Node || name != Some(node_name.as_str()) {
                return;
            }
            if let Some(registry) = bind_registry.upgrade() {
                if let Ok(proxy) = registry.bind::<pw::node::Node, _>(global) {
                    *bind_node.borrow_mut() = Some(proxy);
                    let _ = ready.send(global.id);
                }
            }
        })
        .register();

    let quit = mainloop.clone();
    let _attached = receiver.attach(mainloop.loop_(), move |message| match message {
        Message::Params(params) => {
            if let Some(proxy) = node.borrow().as_ref() {
                let _ = send_params(proxy, &params);
            }
        }
        Message::Quit => quit.quit(),
    });
    mainloop.run();
    Ok(())
}

impl Audio {
    /// `generation` keeps node names unique while a replacement filter starts beside the old one.
    pub fn start(output: &str, sofa: &Path, generation: u32) -> Result<Self> {
        let (sender, receiver) = pw::channel::channel();
        let (ready_sender, ready) = mpsc::channel();
        let node_name = format!("effect_input.soundhold.{generation}");
        let thread = thread::spawn(move || {
            if let Err(error) = run_loop(receiver, ready_sender, node_name) {
                eprintln!("PipeWire connection failed: {error}");
            }
        });
        let mut filter = start_filter(output, sofa, generation)?;
        let node = match ready.recv_timeout(Duration::from_secs(5)) {
            Ok(node) => node,
            Err(_) => {
                let _ = filter.kill();
                let _ = filter.wait();
                let _ = sender.send(Message::Quit);
                let _ = thread.join();
                return Err(anyhow!("the sound filter could not start with this head data"));
            }
        };
        Ok(Self { sender, thread: Some(thread), filter, node, side_angle: DEFAULT_WIDTH, wet_gain: DEFAULT_LEVEL })
    }

    fn send(&self, params: Vec<(String, f32)>) {
        let _ = self.sender.send(Message::Params(params));
    }

    pub fn set_rotation(&self, rotation: Quat) {
        let mut params = Vec::new();
        for (name, azimuth) in [("spC", 0.0), ("spL", self.side_angle), ("spR", -self.side_angle)] {
            let (heard_azimuth, heard_elevation) = sensor::direction(rotation, azimuth);
            params.push((format!("{name}:Azimuth"), heard_azimuth.rem_euclid(360.0) as f32));
            params.push((format!("{name}:Elevation"), heard_elevation as f32));
        }
        self.send(params);
    }

    pub fn set_spatial(&self, enabled: bool) {
        let (wet, dry) = if enabled { (self.wet_gain as f32, 0.0) } else { (0.0, DRY_GAIN) };
        let mut params = Vec::new();
        for output in ["outL", "outR"] {
            for input in 1..=3 {
                params.push((format!("{output}:Gain {input}"), wet));
            }
            params.push((format!("{output}:Gain 4"), dry));
        }
        self.send(params);
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        let _ = self.sender.send(Message::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        unsafe { libc::kill(self.filter.id() as i32, libc::SIGTERM) };
        let _ = self.filter.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_escapes_quotes_and_backslashes() {
        assert_eq!(quoted(r#"/home/a"b\c.sofa"#), r#"/home/a\"b\\c.sofa"#);
    }
}
