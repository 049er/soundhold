use crate::bluetooth::Scan;
use crate::engine::Engine;
use crate::settings::{self, Settings, DEFAULT_LEVEL, DEFAULT_WIDTH};
use crate::{autostart, bluetooth, heads};
use anyhow::{Context, Result};
use ksni::blocking::TrayMethods;
use std::path::Path;
use std::process::Command as Process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use zbus::blocking::{connection, fdo::DBusProxy, Connection};

const BUS_NAME: &str = "io.github.soundhold.Daemon";
const OBJECT_PATH: &str = "/io/github/soundhold/Daemon";
const POLL: Duration = Duration::from_millis(20);
const SEARCH_INTERVAL: Duration = Duration::from_secs(3);
const SAVE_INTERVAL: Duration = Duration::from_secs(1);
const MISSES_BEFORE_GONE: u32 = 2;
const WINDOW_PROGRAM: &str = "soundhold-window";
const INSTALLED_TRAY_ICON: &str = "/usr/share/icons/hicolor/symbolic/apps/soundhold-symbolic.svg";
const DEVELOPMENT_ICON_FOLDER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/icons/hicolor/symbolic/apps");

enum Command {
    SetSpatial(bool),
    SetWidth(f64),
    SetLevel(f64),
    SetHead(String),
    Recentre,
    SetCodec(String),
    Reset,
    Quit,
}

#[derive(Clone, Default)]
struct Status {
    state: String,
    turn: f64,
    height: f64,
    spatial: bool,
    autostart: bool,
    codec: String,
    codecs: Vec<String>,
    error: String,
    width: f64,
    level: f64,
    head: String,
    heads: Vec<String>,
}

#[derive(Clone)]
struct Controls {
    status: Arc<Mutex<Status>>,
    commands: Sender<Command>,
    autostart: Sender<bool>,
}

impl Controls {
    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    fn set_spatial(&self, enabled: bool) {
        self.status.lock().unwrap().spatial = enabled;
        self.send(Command::SetSpatial(enabled));
    }

    fn set_head(&self, name: String) {
        self.status.lock().unwrap().head = name.clone();
        self.send(Command::SetHead(name));
    }
}

struct Service(Controls);

#[zbus::interface(name = "io.github.soundhold.Daemon")]
impl Service {
    #[allow(clippy::type_complexity)]
    fn status(&self) -> (String, f64, f64, bool, bool, String, Vec<String>, String, f64, f64, String, Vec<String>) {
        let s = self.0.status.lock().unwrap().clone();
        (s.state, s.turn, s.height, s.spatial, s.autostart, s.codec, s.codecs, s.error, s.width, s.level, s.head, s.heads)
    }

    fn set_spatial(&self, enabled: bool) {
        self.0.set_spatial(enabled);
    }

    fn set_width(&self, degrees: f64) {
        let degrees = settings::clamp_width(degrees);
        self.0.status.lock().unwrap().width = degrees;
        self.0.send(Command::SetWidth(degrees));
    }

    fn set_level(&self, level: f64) {
        let level = settings::clamp_level(level);
        self.0.status.lock().unwrap().level = level;
        self.0.send(Command::SetLevel(level));
    }

    fn set_head(&self, name: String) {
        if heads::list().contains(&name) {
            self.0.set_head(name);
        }
    }

    fn add_head(&self, path: String) {
        match heads::add(Path::new(&path)) {
            Ok(name) => {
                self.0.status.lock().unwrap().heads = heads::list();
                self.0.set_head(name);
            }
            Err(error) => self.0.status.lock().unwrap().error = error.to_string(),
        }
    }

    fn recentre(&self) {
        self.0.send(Command::Recentre);
    }

    fn set_codec(&self, codec: String) {
        self.0.status.lock().unwrap().codec = codec.clone();
        self.0.send(Command::SetCodec(codec));
    }

    fn reset_defaults(&self) {
        {
            let mut status = self.0.status.lock().unwrap();
            status.width = DEFAULT_WIDTH;
            status.level = DEFAULT_LEVEL;
            status.head = heads::DEFAULT.into();
        }
        self.0.send(Command::Reset);
    }

    fn set_autostart(&self, enabled: bool) {
        self.0.status.lock().unwrap().autostart = enabled;
        let _ = self.0.autostart.send(enabled);
    }

    fn quit(&self) {
        self.0.send(Command::Quit);
    }
}

struct Tray {
    controls: Controls,
    state: String,
    spatial: bool,
}

fn state_label(state: &str) -> &'static str {
    match state {
        "spatial" => "Sound stays in place as you turn",
        "normal" => "Spatial sound is off",
        "reconnecting" => "Connecting to the head sensor",
        "no_access" => "Head sensor needs permission",
        "no_data" => "Head sensor is not responding",
        "unsupported" => "Headphones are not supported",
        "starting" => "Starting up",
        _ => "No headphones connected",
    }
}

fn open_window() {
    let sibling = std::env::current_exe().ok().and_then(|p| Some(p.parent()?.join(WINDOW_PROGRAM)));
    let program = sibling.filter(|p| p.exists()).unwrap_or_else(|| WINDOW_PROGRAM.into());
    if let Ok(mut child) = Process::new(program).spawn() {
        thread::spawn(move || child.wait());
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "soundhold".into()
    }

    fn icon_name(&self) -> String {
        "soundhold-symbolic".into()
    }

    fn icon_theme_path(&self) -> String {
        if !cfg!(debug_assertions) || Path::new(INSTALLED_TRAY_ICON).exists() {
            String::new()
        } else {
            DEVELOPMENT_ICON_FOLDER.into()
        }
    }

    fn title(&self) -> String {
        "Soundhold".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: "Soundhold: sound stays in place".into(), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        open_window();
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{CheckmarkItem, StandardItem};
        vec![
            StandardItem { label: state_label(&self.state).into(), enabled: false, ..Default::default() }.into(),
            ksni::MenuItem::Separator,
            CheckmarkItem {
                label: "Spatial sound".into(),
                checked: self.spatial,
                activate: Box::new(|tray: &mut Self| tray.controls.set_spatial(!tray.spatial)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Centre sound where I'm facing".into(),
                activate: Box::new(|tray: &mut Self| tray.controls.send(Command::Recentre)),
                ..Default::default()
            }
            .into(),
            StandardItem { label: "Open Soundhold".into(), activate: Box::new(|_: &mut Self| open_window()), ..Default::default() }.into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit Soundhold".into(),
                activate: Box::new(|tray: &mut Self| tray.controls.send(Command::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

struct Watch {
    address: Mutex<Option<String>>,
    unsupported: AtomicBool,
}

fn watch_headphones() -> Arc<Watch> {
    let watch = Arc::new(Watch { address: Mutex::new(None), unsupported: AtomicBool::new(false) });
    let shared = watch.clone();
    thread::spawn(move || {
        let mut misses = 0;
        loop {
            let scan = bluetooth::scan();
            shared.unsupported.store(scan == Scan::Unsupported, Ordering::Relaxed);
            if let Scan::Supported(address) = scan {
                misses = 0;
                *shared.address.lock().unwrap() = Some(address);
            } else {
                misses += 1;
                if misses >= MISSES_BEFORE_GONE {
                    *shared.address.lock().unwrap() = None;
                }
            }
            thread::sleep(SEARCH_INTERVAL);
        }
    });
    watch
}

/// A saved codec may not exist on other headphones, so a failure falls back to the default codec.
fn start_engine(address: &str, settings: &Settings, spatial: bool) -> Result<Engine> {
    let sofa = heads::resolve(settings.head.as_deref())?;
    let mut engine = Engine::start(address, settings.codec.as_deref(), true, &sofa).or_else(|error| {
        if settings.codec.is_some() {
            Engine::start(address, None, true, &sofa)
        } else {
            Err(error)
        }
    })?;
    engine.set_width(settings.width);
    engine.set_level(settings.level);
    engine.set_spatial(spatial)?;
    Ok(engine)
}

fn spawn_autostart_worker(status: Arc<Mutex<Status>>) -> Sender<bool> {
    let (sender, receiver) = mpsc::channel::<bool>();
    thread::spawn(move || {
        while let Ok(first) = receiver.recv() {
            let enabled = receiver.try_iter().last().unwrap_or(first);
            if enabled == autostart::enabled() {
                continue;
            }
            if let Err(error) = autostart::set(enabled) {
                let mut published = status.lock().unwrap();
                published.autostart = autostart::enabled();
                published.error = error.to_string();
            }
        }
    });
    sender
}

pub fn is_running() -> Result<bool> {
    let connection = Connection::session()?;
    Ok(DBusProxy::new(&connection)?.name_has_owner(BUS_NAME.try_into()?)?)
}

pub fn stop() -> Result<()> {
    Connection::session()?.call_method(Some(BUS_NAME), OBJECT_PATH, Some(BUS_NAME), "Quit", &()).context("the background program is not running")?;
    Ok(())
}

fn save(settings: &Settings) {
    if let Err(error) = settings.save() {
        eprintln!("could not save the settings: {error}");
    }
}

pub fn run() -> Result<()> {
    crate::install_signals();
    let mut settings = Settings::load();
    let status = Arc::new(Mutex::new(Status {
        state: "waiting".into(),
        spatial: true,
        autostart: autostart::enabled(),
        width: settings.width,
        level: settings.level,
        head: settings.head.clone().unwrap_or_else(|| heads::DEFAULT.into()),
        heads: heads::list(),
        ..Default::default()
    }));
    let (commands, received) = mpsc::channel();
    let controls = Controls { status: status.clone(), commands, autostart: spawn_autostart_worker(status.clone()) };
    let _connection = connection::Builder::session()?
        .name(BUS_NAME)?
        .allow_name_replacements(false)
        .replace_existing_names(false)
        .serve_at(OBJECT_PATH, Service(controls.clone()))?
        .build()
        .context("could not start, is the background program already running?")?;
    let tray = match (Tray { controls, state: "waiting".into(), spatial: true }).spawn() {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("no tray icon: {error}");
            None
        }
    };

    let headphones = watch_headphones();
    let mut engine: Option<Engine> = None;
    let mut spatial = true;
    let mut next_attempt = Instant::now();
    let mut shown = (String::from("waiting"), true);
    let mut unsaved = false;
    let mut last_save = Instant::now();

    'main: while !crate::stop_requested() {
        let mut failure = None;
        let mut received_command = false;
        let (mut wanted_spatial, mut wanted_width, mut wanted_level, mut wanted_head) = (None, None, None, None);
        for command in received.try_iter() {
            received_command = true;
            match command {
                Command::Quit => break 'main,
                Command::Recentre => {
                    if let Some(running) = engine.as_mut() {
                        running.recentre();
                    }
                }
                Command::SetSpatial(enabled) => wanted_spatial = Some(enabled),
                Command::SetWidth(degrees) => wanted_width = Some(degrees),
                Command::SetLevel(level) => wanted_level = Some(level),
                Command::SetHead(name) => wanted_head = Some(name),
                Command::SetCodec(name) => {
                    if let Some(running) = engine.as_mut() {
                        failure = running.set_codec(Some(&name)).err().or(failure);
                    }
                    settings.codec = Some(name);
                    unsaved = true;
                }
                Command::Reset => {
                    wanted_width = Some(DEFAULT_WIDTH);
                    wanted_level = Some(DEFAULT_LEVEL);
                    wanted_head = Some(heads::DEFAULT.to_string());
                    settings.codec = None;
                    unsaved = true;
                    if let Some(running) = engine.as_mut() {
                        failure = running.set_codec(None).err().or(failure);
                    }
                }
            }
        }
        if let Some(degrees) = wanted_width {
            settings.width = degrees;
            unsaved = true;
            if let Some(running) = engine.as_mut() {
                running.set_width(degrees);
            }
        }
        if let Some(level) = wanted_level {
            settings.level = level;
            unsaved = true;
            if let Some(running) = engine.as_mut() {
                running.set_level(level);
            }
        }
        if let Some(name) = wanted_head {
            let current = settings.head.clone().unwrap_or_else(|| heads::DEFAULT.into());
            if name != current {
                let applied = match engine.as_mut() {
                    Some(running) => heads::resolve(Some(&name)).and_then(|sofa| running.set_head(&sofa)),
                    None => Ok(()),
                };
                match applied {
                    Ok(()) => {
                        settings.head = Some(name);
                        unsaved = true;
                    }
                    Err(error) => {
                        status.lock().unwrap().head = current;
                        failure = Some(error);
                    }
                }
            }
        }
        if let Some(enabled) = wanted_spatial {
            spatial = enabled;
            if let Some(running) = engine.as_mut() {
                failure = running.set_spatial(enabled).err().or(failure);
            }
        }

        let address = headphones.address.lock().unwrap().clone();
        match address {
            None => engine = None,
            Some(address) if engine.is_none() && Instant::now() >= next_attempt => {
                next_attempt = Instant::now() + SEARCH_INTERVAL;
                status.lock().unwrap().state = "starting".into();
                match start_engine(&address, &settings, spatial) {
                    Ok(started) => {
                        status.lock().unwrap().error.clear();
                        engine = Some(started);
                    }
                    Err(error) => failure = Some(error),
                }
            }
            Some(_) => {}
        }

        let reading = engine.as_mut().map(Engine::tick);
        {
            let mut published = status.lock().unwrap();
            let idle = if headphones.unsupported.load(Ordering::Relaxed) { "unsupported" } else { "waiting" };
            published.state = reading.as_ref().map_or(idle, |r| r.state).into();
            published.turn = reading.as_ref().map_or(0.0, |r| r.turn);
            published.height = reading.as_ref().map_or(0.0, |r| r.height);
            if let Some(running) = &engine {
                published.codec = running.codec().unwrap_or_default().into();
                published.codecs = running.codecs().to_vec();
            }
            match failure {
                Some(error) => published.error = error.to_string(),
                None if received_command => published.error.clear(),
                None => {}
            }
            let current = (published.state.clone(), published.spatial);
            if current != shown {
                if let Some(handle) = &tray {
                    handle.update(|tray| (tray.state, tray.spatial) = current.clone());
                }
                shown = current;
            }
        }
        if unsaved && last_save.elapsed() >= SAVE_INTERVAL {
            save(&settings);
            unsaved = false;
            last_save = Instant::now();
        }
        thread::sleep(POLL);
    }
    if unsaved {
        save(&settings);
    }
    drop(engine);
    if let Some(handle) = tray {
        handle.shutdown().wait();
    }
    Ok(())
}
