mod audio;
mod autostart;
mod bluetooth;
mod daemon;
mod engine;
mod heads;
mod sensor;
mod session;
mod settings;

use anyhow::{anyhow, Result};
use std::io::{IsTerminal, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const POLL_MS: i32 = 20;
const USAGE: &str = "usage: soundhold [run [--seconds N] [--codec NAME] | daemon | stop | autostart on|off | check]";

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn request_stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

pub fn install_signals() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        unsafe { libc::signal(signal, request_stop as *const () as libc::sighandler_t) };
    }
}

pub fn stop_requested() -> bool {
    STOP.load(Ordering::Relaxed)
}

struct RawTerminal(libc::termios);

impl RawTerminal {
    fn enter() -> Self {
        unsafe {
            let mut settings: libc::termios = std::mem::zeroed();
            libc::tcgetattr(libc::STDIN_FILENO, &mut settings);
            let saved = settings;
            settings.c_lflag &= !(libc::ICANON | libc::ECHO);
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &settings);
            Self(saved)
        }
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.0) };
        println!();
    }
}

fn read_key(timeout_ms: i32) -> Option<u8> {
    let mut poll = libc::pollfd { fd: libc::STDIN_FILENO, events: libc::POLLIN, revents: 0 };
    if unsafe { libc::poll(&mut poll, 1, timeout_ms) } <= 0 {
        return None;
    }
    let mut key = [0u8; 1];
    matches!(std::io::stdin().read(&mut key), Ok(1)).then_some(key[0])
}

fn terminal_width() -> usize {
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) };
    if result == 0 && size.ws_col > 1 { size.ws_col as usize - 1 } else { 79 }
}

fn run(seconds: Option<f64>, codec: Option<&str>) -> Result<()> {
    if daemon::is_running()? {
        return Err(anyhow!("the background program is running, stop it first with: soundhold stop"));
    }
    let address = bluetooth::find_headphones().ok_or_else(|| anyhow!("no connected headphones with a head tracker service found"))?;
    let interactive = std::io::stdin().is_terminal();
    install_signals();

    let mut engine = engine::Engine::start(&address, codec, interactive, &heads::resolve(None)?)?;
    let _terminal = interactive.then(RawTerminal::enter);
    let started = Instant::now();
    let mut last_status = Instant::now() - Duration::from_secs(1);

    while !stop_requested() && seconds.is_none_or(|s| started.elapsed().as_secs_f64() < s) {
        let key = if interactive {
            read_key(POLL_MS)
        } else {
            std::thread::sleep(Duration::from_millis(POLL_MS as u64));
            None
        };
        match key {
            Some(b'q') => break,
            Some(b'c') => engine.recentre(),
            Some(b'b') => engine.set_spatial(!engine.spatial())?,
            _ => {}
        }
        let reading = engine.tick();
        if last_status.elapsed() >= Duration::from_millis(100) {
            last_status = Instant::now();
            let status = format!("{:12} turn {:6.1} height {:6.1}  c centre, b compare, q quit", reading.state, reading.turn, reading.height);
            if interactive {
                let width = terminal_width().min(status.len());
                print!("\r\x1b[K{}", &status[..width]);
                let _ = std::io::stdout().flush();
            } else if started.elapsed().as_secs_f64() > 0.5 {
                println!("{status}");
            }
        }
    }
    if !interactive {
        println!("sound updates: {}", engine.updates);
    }
    Ok(())
}

fn run_with_options(options: &[&str]) -> Result<()> {
    let (mut seconds, mut codec) = (None, None);
    for option in options.chunks(2) {
        match option {
            ["--seconds", value] => seconds = Some(value.parse()?),
            ["--codec", value] => codec = Some(*value),
            _ => return Err(anyhow!(USAGE)),
        }
    }
    run(seconds, codec)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => run(None, None),
        ["run", options @ ..] => run_with_options(options),
        ["daemon"] => daemon::run(),
        ["stop"] => daemon::stop(),
        ["autostart", "on"] => autostart::set(true),
        ["autostart", "off"] => autostart::set(false),
        ["check"] => sensor::check(),
        _ => Err(anyhow!(USAGE)),
    }
}
