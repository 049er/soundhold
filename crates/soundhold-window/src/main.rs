mod theme;

use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

const APP_ID: &str = "io.github.soundhold.Window";
const BUS_NAME: &str = "io.github.soundhold.Daemon";
const OBJECT_PATH: &str = "/io/github/soundhold/Daemon";
const DAEMON_PROGRAM: &str = "soundhold";
const ISSUES_URL: &str = "https://github.com/049er/soundhold/issues/new";
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const CALL_TIMEOUT_MS: i32 = 1000;
const WINDOW_WIDTH: i32 = 400;
const INPUT_HOLD: Duration = Duration::from_millis(500);
const ENABLE_HOLD: Duration = Duration::from_secs(5);
const ENABLE_RETRY_POLLS: u32 = 5;
const SLIDER_TOLERANCE: f64 = 0.5;
const HEAD_LABEL_LENGTH: usize = 18;
const ADD_HEAD: &str = "add-your-own";

type Reply = (String, f64, f64, bool, bool, String, Vec<String>, String, f64, f64, String, Vec<String>, String);

#[derive(Clone, Default)]
struct Snapshot {
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
    device: String,
}

impl From<Reply> for Snapshot {
    fn from(r: Reply) -> Self {
        Self { state: r.0, turn: r.1, height: r.2, spatial: r.3, autostart: r.4, codec: r.5, codecs: r.6, error: r.7, width: r.8, level: r.9, head: r.10, heads: r.11, device: r.12 }
    }
}

/// Ignores status refreshes for a short time after the user changes a control.
#[derive(Default)]
struct InputHold(Cell<Option<Instant>>);

impl InputHold {
    fn start(&self, length: Duration) {
        self.0.set(Some(Instant::now() + length));
    }

    fn active(&self) -> bool {
        self.0.get().is_some_and(|until| Instant::now() < until)
    }
}

struct Toggle {
    control: gtk::Switch,
    hold: InputHold,
}

impl Toggle {
    fn new() -> Self {
        let control = gtk::Switch::new();
        control.add_css_class("m3-switch");
        Self { control, hold: InputHold::default() }
    }

    /// Setting a switch during its click animation cancels the click, so it is only set when different.
    fn sync(&self, value: bool) {
        if !self.hold.active() && self.control.is_active() != value {
            self.control.set_active(value);
        }
    }
}

struct Slider {
    control: gtk::Scale,
    value: gtk::Label,
    hold: InputHold,
    scale: f64,
}

impl Slider {
    fn new(range: (f64, f64), scale: f64) -> Self {
        let control = gtk::Scale::with_range(gtk::Orientation::Horizontal, range.0, range.1, 1.0);
        control.add_css_class("m3-slider");
        control.set_round_digits(0);
        control.set_hexpand(true);
        Self { control, value: label("", "m3-body-medium"), hold: InputHold::default(), scale }
    }

    fn sync(&self, reported: f64) {
        let value = reported * self.scale;
        if !self.hold.active() && (self.control.value() - value).abs() > SLIDER_TOLERANCE {
            self.control.set_value(value);
        }
    }
}

struct Choice {
    control: gtk::DropDown,
    items: RefCell<Vec<String>>,
    reported: RefCell<Option<String>>,
    display: fn(&str) -> String,
}

impl Choice {
    fn new(display: fn(&str) -> String) -> Self {
        let control = gtk::DropDown::from_strings(&[]);
        control.add_css_class("m3-dropdown");
        Self { control, items: RefCell::default(), reported: RefCell::default(), display }
    }

    fn sync(&self, items: &[String], current: &str) {
        if *self.items.borrow() != items {
            let names: Vec<String> = items.iter().map(|item| (self.display)(item)).collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            self.control.set_model(Some(&gtk::StringList::new(&names)));
            *self.items.borrow_mut() = items.to_vec();
            self.reported.take();
        }
        if self.reported.borrow().as_deref() != Some(current) {
            if let Some(index) = items.iter().position(|item| item == current) {
                self.control.set_selected(index as u32);
            }
            self.reported.replace(Some(current.to_string()));
        }
    }

    fn selected(&self) -> Option<String> {
        self.items.borrow().get(self.control.selected() as usize).cloned()
    }

    fn select_reported(&self) {
        let reported = self.reported.borrow();
        if let Some(index) = self.items.borrow().iter().position(|item| Some(item) == reported.as_ref()) {
            self.control.set_selected(index as u32);
        }
    }
}

struct Ui {
    bus: gio::DBusConnection,
    updating: Cell<bool>,
    polls: Cell<u32>,
    latest: RefCell<Snapshot>,
    state: gtk::Label,
    detail: gtk::Label,
    error: gtk::Label,
    position: gtk::Label,
    centre: gtk::Button,
    spatial: Toggle,
    width: Slider,
    level: Slider,
    head: Choice,
    codec: Choice,
    autostart: Toggle,
    enable: Toggle,
    start: gtk::Button,
    report_headphones: gtk::Button,
}

impl Ui {
    fn call(self: &Rc<Self>, method: &str, arguments: Option<&glib::Variant>) {
        let ui = self.clone();
        self.bus.call(
            Some(BUS_NAME),
            OBJECT_PATH,
            BUS_NAME,
            method,
            arguments,
            None,
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
            None::<&gio::Cancellable>,
            move |reply| {
                if reply.is_err() {
                    ui.codec.reported.take();
                    ui.head.reported.take();
                }
            },
        );
    }

    fn refresh(self: &Rc<Self>) {
        let ui = self.clone();
        self.bus.call(
            Some(BUS_NAME),
            OBJECT_PATH,
            BUS_NAME,
            "Status",
            None,
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            CALL_TIMEOUT_MS,
            None::<&gio::Cancellable>,
            move |reply| ui.show(reply.ok().and_then(|value| value.get::<Reply>())),
        );
    }

    fn show(&self, reply: Option<Reply>) {
        self.updating.set(true);
        let running = reply.is_some();
        let snapshot = reply.map(Snapshot::from).unwrap_or_default();
        self.latest.replace(snapshot.clone());
        let Snapshot { state, turn, height, spatial, autostart, codec, codecs, error, width, level, head, heads, .. } = snapshot;
        let (title, detail) = match (running, state.as_str()) {
            (false, _) => ("Stopped", "The app is disabled"),
            (_, "spatial") => ("Head tracking is on", "Sound stays in place as you turn"),
            (_, "normal") => ("Normal sound", "Head tracking is off"),
            (_, "reconnecting") => ("Connecting to the sensor", "Sound plays but will not follow your head yet"),
            (_, "no_access") => ("No access to the head sensor", "Sound plays but will not follow your head. Run install-rule.sh or reinstall Soundhold"),
            (_, "no_data") => ("Head sensor is not responding", "Sound plays but will not follow your head. Check the earbuds are in and head tracking is on in the headphone app"),
            (_, "no_sensor") => ("No head sensor found", "Sound plays but will not follow your head. These headphones may not be supported yet"),
            (_, "unsupported") => ("Headphones not supported", "The connected headphones have no head sensor Soundhold can use"),
            (_, "starting") => ("Starting", "Setting up the sound"),
            _ => ("Waiting for headphones", "Connect your headphones"),
        };
        self.state.set_label(title);
        self.detail.set_label(detail);
        self.error.set_label(&error);
        self.error.set_visible(!error.is_empty());
        let tracking = state == "spatial";
        self.report_headphones.set_visible(matches!(state.as_str(), "no_sensor" | "no_data" | "unsupported"));
        self.position.set_label(&if tracking { format!("Turn {}   Height {}", degrees(turn), degrees(height)) } else { "Not tracking".to_string() });
        self.centre.set_sensitive(tracking);
        for control in [
            self.spatial.control.upcast_ref::<gtk::Widget>(),
            self.autostart.control.upcast_ref(),
            self.width.control.upcast_ref(),
            self.level.control.upcast_ref(),
            self.head.control.upcast_ref(),
        ] {
            control.set_sensitive(running);
        }
        self.codec.control.set_sensitive(!codecs.is_empty());
        self.start.set_visible(!running && !self.enable.hold.active());
        self.enable.sync(running);
        self.retry_start(running);
        if running {
            self.spatial.sync(spatial);
            self.autostart.sync(autostart);
            self.width.sync(width);
            self.level.sync(level);
            let mut choices = heads;
            choices.push(ADD_HEAD.to_string());
            self.head.sync(&choices, &head);
        }
        self.codec.sync(&codecs, &codec);
        self.updating.set(false);
    }

    /// A previous copy can take a few seconds to exit, and a new one cannot start until it has.
    fn retry_start(&self, running: bool) {
        let polls = self.polls.get().wrapping_add(1);
        self.polls.set(polls);
        if !running && self.enable.hold.active() && self.enable.control.is_active() && polls.is_multiple_of(ENABLE_RETRY_POLLS) {
            start_daemon();
        }
    }

    fn open_issue(&self, window: &gtk::ApplicationWindow, template: &str, title: &str, fields: &[(&str, &str)]) {
        gtk::UriLauncher::new(&issue_url(template, title, fields)).launch(Some(window), None::<&gio::Cancellable>, |_| {});
    }

    fn report_issue(&self, window: &gtk::ApplicationWindow) {
        let s = self.latest.borrow();
        let details = issue_details(&s.state, &s.codec, &s.error);
        self.open_issue(window, "bug_report.yml", "", &[("system", &system_name()), ("version", env!("CARGO_PKG_VERSION")), ("details", &details)]);
    }

    fn report_headphones(&self, window: &gtk::ApplicationWindow) {
        let s = self.latest.borrow();
        let details = headphones_details(env!("CARGO_PKG_VERSION"), &system_name(), &s.device, &s.state);
        self.open_issue(window, "headphones_report.yml", "Headphones report: ", &[("details", &details)]);
    }

    fn share_setup(&self, window: &gtk::ApplicationWindow) {
        let s = self.latest.borrow();
        let settings = setup_details(env!("CARGO_PKG_VERSION"), &system_name(), &s.head, s.width, s.level, &s.codec);
        self.open_issue(window, "setup_report.yml", "Setup report: ", &[("settings", &settings)]);
    }
}

fn degrees(value: f64) -> String {
    format!("{:.0}°", value.round() + 0.0)
}

fn codec_label(name: &str) -> String {
    name.to_uppercase().replace('_', "-")
}

fn head_label(name: &str) -> String {
    match name {
        "HRIR_FULL2DEG.sofa" => "KU100 full sphere".to_string(),
        "fhk_circ360.sofa" => "KU100 level only".to_string(),
        ADD_HEAD => "+ Add your own file".to_string(),
        _ => {
            let stem = name.strip_suffix(".sofa").unwrap_or(name);
            if stem.chars().count() > HEAD_LABEL_LENGTH {
                format!("{}…", stem.chars().take(HEAD_LABEL_LENGTH - 1).collect::<String>())
            } else {
                stem.to_string()
            }
        }
    }
}

fn system_name() -> String {
    let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    release
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .map_or_else(|| "unknown".to_string(), |name| name.trim_matches('"').to_string())
}

/// `fields` fills the issue form fields with these ids in `.github/ISSUE_TEMPLATE/<template>`.
fn issue_url(template: &str, title: &str, fields: &[(&str, &str)]) -> String {
    let escape = |text: &str| glib::Uri::escape_string(text, None, false);
    let mut url = format!("{ISSUES_URL}?template={}&title={}", escape(template), escape(title));
    for (id, value) in fields {
        url.push_str(&format!("&{}={}", escape(id), escape(value)));
    }
    url
}

fn issue_details(state: &str, codec: &str, error: &str) -> String {
    format!("State: {state}\nCodec: {codec}\nLast error: {error}\n")
}

fn headphones_details(version: &str, system: &str, device: &str, state: &str) -> String {
    format!("{device}State: {state}\nVersion: {version}\nSystem: {system}\n")
}

fn setup_details(version: &str, system: &str, head: &str, width: f64, level: f64, codec: &str) -> String {
    format!(
        "Head data: {head}\nSound width: {width:.0}°\nSpatial level: {:.0}%\nCodec: {codec}\nVersion: {version}\nSystem: {system}\n",
        level * 100.0
    )
}

fn start_daemon() {
    let sibling = std::env::current_exe().ok().and_then(|p| Some(p.parent()?.join(DAEMON_PROGRAM)));
    let program = sibling.filter(|p| p.exists()).unwrap_or_else(|| DAEMON_PROGRAM.into());
    let started = Command::new(program)
        .arg("daemon")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = started {
        std::thread::spawn(move || child.wait());
    }
}

fn label(text: &str, class: &str) -> gtk::Label {
    let widget = gtk::Label::builder().label(text).xalign(0.0).build();
    widget.add_css_class(class);
    widget
}

fn card() -> gtk::Box {
    let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
    widget.add_css_class("m3-card");
    widget
}

fn divider() -> gtk::Box {
    let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    widget.add_css_class("m3-divider");
    widget
}

fn setting_row(title: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    let name = label(title, "m3-body-large");
    name.set_hexpand(true);
    row.append(&name);
    control.set_valign(gtk::Align::Center);
    row.append(control);
    row
}

fn slider_row(title: &str, slider: &Slider) -> gtk::Box {
    let header = setting_row(title, &slider.value);
    let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    row.append(&header);
    row.append(&slider.control);
    row
}

fn compact_button(text: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::with_label(text);
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("m3-outlined-button");
    button.add_css_class("m3-compact-button");
    button
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(tooltip));
    button.set_valign(gtk::Align::Center);
    button.add_css_class("m3-icon-button");
    button
}

fn titlebar(window: &gtk::ApplicationWindow) -> gtk::WindowHandle {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.add_css_class("m3-titlebar");
    let title = label("Soundhold", "m3-title-bar-text");
    title.set_hexpand(true);
    bar.append(&title);

    let minimize = icon_button("window-minimize-symbolic", "Minimize");
    let maximize = icon_button("window-maximize-symbolic", "Maximize");
    let close = icon_button("window-close-symbolic", "Close");
    for button in [&minimize, &maximize, &close] {
        bar.append(button);
    }
    let target = window.clone();
    minimize.connect_clicked(move |_| target.minimize());
    let target = window.clone();
    maximize.connect_clicked(move |_| if target.is_maximized() { target.unmaximize() } else { target.maximize() });
    let target = window.clone();
    close.connect_clicked(move |_| target.close());
    window.connect_maximized_notify(move |target| {
        maximize.set_icon_name(if target.is_maximized() { "window-restore-symbolic" } else { "window-maximize-symbolic" });
    });

    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&bar));
    handle
}

fn apply_theme(provider: &gtk::CssProvider) {
    let manager = adw::StyleManager::default();
    provider.load_from_string(&theme::stylesheet(manager.accent_color_rgba(), manager.is_dark()));
}

fn install_theme() {
    let provider = gtk::CssProvider::new();
    apply_theme(&provider);
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("no display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let manager = adw::StyleManager::default();
    let target = provider.clone();
    manager.connect_accent_color_rgba_notify(move |_| apply_theme(&target));
    manager.connect_dark_notify(move |_| apply_theme(&provider));
}

fn status_card(ui: &Ui) -> gtk::Box {
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    text.append(&ui.state);
    text.append(&ui.detail);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    top.append(&text);
    ui.spatial.control.set_valign(gtk::Align::Center);
    top.append(&ui.spatial.control);

    let status = card();
    status.set_spacing(12);
    status.append(&top);
    status.append(&ui.error);
    status.append(&ui.centre);
    status.append(&ui.position);
    status.append(&ui.report_headphones);
    status
}

fn advanced_section(ui: &Ui, reset: &gtk::Button) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 12);
    section.add_css_class("m3-advanced");
    section.append(&slider_row("Sound width", &ui.width));
    section.append(&slider_row("Spatial level", &ui.level));
    section.append(&setting_row("Head data", &ui.head.control));
    section.append(&setting_row("Bluetooth codec", &ui.codec.control));
    section.append(&divider());
    section.append(&setting_row("Start at login", &ui.autostart.control));
    section.append(&setting_row("Enable app", &ui.enable.control));
    section.append(&divider());
    reset.set_halign(gtk::Align::End);
    section.append(reset);
    section
}

fn connect_toggle(ui: &Rc<Ui>, toggle: impl Fn(&Ui) -> &Toggle + 'static, method: &'static str) {
    let target = ui.clone();
    toggle(ui).control.connect_active_notify(move |control| {
        if !target.updating.get() {
            toggle(&target).hold.start(INPUT_HOLD);
            target.call(method, Some(&(control.is_active(),).to_variant()));
        }
    });
}

fn connect_slider(ui: &Rc<Ui>, slider: impl Fn(&Ui) -> &Slider + 'static, method: &'static str, format: fn(f64) -> String) {
    let target = ui.clone();
    slider(ui).control.connect_value_changed(move |control| {
        let current = slider(&target);
        current.value.set_label(&format(control.value()));
        if !target.updating.get() {
            current.hold.start(INPUT_HOLD);
            target.call(method, Some(&(control.value() / current.scale,).to_variant()));
        }
    });
}

fn connect_choice(ui: &Rc<Ui>, choice: impl Fn(&Ui) -> &Choice + 'static, method: &'static str) {
    let target = ui.clone();
    choice(ui).control.connect_selected_notify(move |_| {
        if target.updating.get() {
            return;
        }
        if let Some(name) = choice(&target).selected() {
            target.call(method, Some(&(name.as_str(),).to_variant()));
        }
    });
}

fn choose_head_file(ui: Rc<Ui>, window: &gtk::ApplicationWindow) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Head data (.sofa)"));
    filter.add_pattern("*.sofa");
    let dialog = gtk::FileDialog::builder().title("Add head data").default_filter(&filter).build();
    dialog.open(Some(window), None::<&gio::Cancellable>, move |chosen| {
        if let Some(path) = chosen.ok().and_then(|file| file.path()) {
            ui.call("AddHead", Some(&(path.to_string_lossy().as_ref(),).to_variant()));
        }
    });
}

fn build_window(app: &adw::Application, bus: gio::DBusConnection) {
    let window = gtk::ApplicationWindow::builder().application(app).title("Soundhold").default_width(WINDOW_WIDTH).build();
    window.add_css_class("m3");
    window.set_titlebar(Some(&titlebar(&window)));

    let centre = gtk::Button::with_label("Set centre");
    centre.add_css_class("m3-tonal-button");
    let start = gtk::Button::with_label("Enable app");
    start.add_css_class("m3-filled-button");
    start.set_visible(false);

    let report_headphones = gtk::Button::with_label("Report these headphones");
    report_headphones.add_css_class("m3-outlined-button");
    report_headphones.set_visible(false);

    let header_text = label("Advanced", "m3-body-large");
    header_text.set_hexpand(true);
    let chevron = gtk::Image::from_icon_name("go-down-symbolic");
    let header_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    header_row.append(&header_text);
    header_row.append(&chevron);
    let advanced = gtk::Button::builder().child(&header_row).build();
    advanced.add_css_class("m3-expansion-header");

    let report = compact_button("Report issue", "Report a problem on GitHub");
    let share = compact_button("Share setup", "Share what works best on your headphones");
    let reset = gtk::Button::with_label("Reset to defaults");
    reset.set_tooltip_text(Some("Reset sound width, level, head data and codec"));
    reset.add_css_class("m3-outlined-button");
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for button in [&report, &share] {
        button.set_hexpand(true);
        actions.append(button);
    }

    let ui = Rc::new(Ui {
        bus,
        updating: Cell::new(false),
        polls: Cell::new(0),
        latest: RefCell::default(),
        state: label("", "m3-title-medium"),
        detail: label("", "m3-body-medium"),
        error: label("", "m3-error"),
        position: label("", "m3-body-small"),
        centre: centre.clone(),
        spatial: Toggle::new(),
        width: Slider::new((10.0, 90.0), 1.0),
        level: Slider::new((50.0, 100.0), 100.0),
        head: Choice::new(head_label),
        codec: Choice::new(codec_label),
        autostart: Toggle::new(),
        enable: Toggle::new(),
        start: start.clone(),
        report_headphones: report_headphones.clone(),
    });
    ui.error.set_wrap(true);
    ui.error.set_visible(false);
    ui.position.add_css_class("m3-tabular");
    ui.position.set_xalign(0.5);
    ui.detail.set_wrap(true);
    ui.detail.set_max_width_chars(33);

    let revealer = gtk::Revealer::builder().transition_type(gtk::RevealerTransitionType::SlideDown).child(&advanced_section(&ui, &reset)).build();
    let expansion = gtk::Box::new(gtk::Orientation::Vertical, 0);
    expansion.add_css_class("m3-expansion");
    expansion.append(&advanced);
    expansion.append(&revealer);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.add_css_class("m3-content");
    content.append(&status_card(&ui));
    content.append(&start);
    content.append(&expansion);
    content.append(&actions);
    window.set_child(Some(&content));

    let target = revealer.clone();
    advanced.connect_clicked(move |_| {
        let open = !target.reveals_child();
        target.set_reveal_child(open);
        chevron.set_icon_name(Some(if open { "go-up-symbolic" } else { "go-down-symbolic" }));
    });
    let parent = window.clone();
    revealer.connect_child_revealed_notify(move |control| {
        if !control.is_child_revealed() {
            parent.set_default_size(WINDOW_WIDTH, -1);
        }
    });

    connect_toggle(&ui, |ui| &ui.spatial, "SetSpatial");
    connect_toggle(&ui, |ui| &ui.autostart, "SetAutostart");
    connect_slider(&ui, |ui| &ui.width, "SetWidth", |value| format!("{value:.0}°"));
    connect_slider(&ui, |ui| &ui.level, "SetLevel", |value| format!("{value:.0}%"));
    connect_choice(&ui, |ui| &ui.codec, "SetCodec");
    let (target, parent) = (ui.clone(), window.clone());
    ui.head.control.connect_selected_notify(move |_| {
        if target.updating.get() {
            return;
        }
        match target.head.selected().as_deref() {
            Some(ADD_HEAD) => {
                target.updating.set(true);
                target.head.select_reported();
                target.updating.set(false);
                choose_head_file(target.clone(), &parent);
            }
            Some(name) if target.head.reported.borrow().as_deref() != Some(name) => target.call("SetHead", Some(&(name,).to_variant())),
            _ => {}
        }
    });
    let target = ui.clone();
    ui.enable.control.connect_active_notify(move |control| {
        if target.updating.get() {
            return;
        }
        target.enable.hold.start(ENABLE_HOLD);
        if control.is_active() {
            start_daemon();
        } else {
            target.call("Quit", None);
        }
    });
    let target = ui.clone();
    centre.connect_clicked(move |_| target.call("Recentre", None));
    let target = ui.clone();
    start.connect_clicked(move |_| target.enable.control.set_active(true));
    let target = ui.clone();
    reset.connect_clicked(move |_| target.call("ResetDefaults", None));
    let (target, parent) = (ui.clone(), window.clone());
    report.connect_clicked(move |_| target.report_issue(&parent));
    let (target, parent) = (ui.clone(), window.clone());
    report_headphones.connect_clicked(move |_| target.report_headphones(&parent));
    let (target, parent) = (ui.clone(), window.clone());
    share.connect_clicked(move |_| target.share_setup(&parent));

    let weak = Rc::downgrade(&ui);
    glib::timeout_add_local(POLL_INTERVAL, move || match weak.upgrade() {
        Some(ui) => {
            ui.refresh();
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    ui.updating.set(true);
    ui.enable.control.set_active(true);
    ui.updating.set(false);
    ui.enable.hold.start(ENABLE_HOLD);
    let holder = RefCell::new(Some(ui));
    window.connect_destroy(move |_| drop(holder.borrow_mut().take()));

    start_daemon();
    window.present();
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| install_theme());
    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        match gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) {
            Ok(bus) => build_window(app, bus),
            Err(error) => eprintln!("no session bus: {error}"),
        }
    });
    app.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_url_names_the_template_and_escapes_the_fields() {
        assert_eq!(
            issue_url("setup_report.yml", "Setup report: ", &[("settings", "A b\nC"), ("version", "1.0")]),
            "https://github.com/049er/soundhold/issues/new?template=setup_report.yml&title=Setup%20report%3A%20&settings=A%20b%0AC&version=1.0"
        );
    }

    #[test]
    fn details_list_the_current_state() {
        for (details, expected) in [
            (issue_details("running", "aac", "none"), "State: running\nCodec: aac\nLast error: none\n"),
            (
                headphones_details("1.0", "Fedora", "Bluetooth IDs: usb:v1234\n", "no_sensor"),
                "Bluetooth IDs: usb:v1234\nState: no_sensor\nVersion: 1.0\nSystem: Fedora\n",
            ),
            (
                setup_details("1.0", "Ubuntu 26.04", "mine.sofa", 45.0, 0.9, "aac"),
                "Head data: mine.sofa\nSound width: 45°\nSpatial level: 90%\nCodec: aac\nVersion: 1.0\nSystem: Ubuntu 26.04\n",
            ),
        ] {
            assert_eq!(details, expected);
        }
    }

    #[test]
    fn head_label_names_included_data_and_shortens_long_names() {
        for (name, expected) in [
            ("HRIR_FULL2DEG.sofa", "KU100 full sphere"),
            ("mine.sofa", "mine"),
            (ADD_HEAD, "+ Add your own file"),
            ("a_very_long_head_data_file.sofa", "a_very_long_head_…"),
        ] {
            assert_eq!(head_label(name), expected);
        }
    }

    #[test]
    fn degrees_never_shows_negative_zero() {
        for (value, expected) in [(-0.3, "0°"), (4.4, "4°"), (-2.6, "-3°")] {
            assert_eq!(degrees(value), expected);
        }
    }
}
