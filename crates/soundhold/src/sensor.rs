use crate::bluetooth;
use anyhow::{anyhow, Context, Result};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const TRACKER_DESCRIPTOR: [u8; 4] = [0x05, 0x20, 0x09, 0xE1];
const MARKER: &[u8] = b"#AndroidHeadTracker#";
const SENSOR_ON: u8 = 0x03;
const SENSOR_OFF: u8 = 0x00;
const RECONNECT_INTERVAL: Duration = Duration::from_secs(3);
const PAUSE_STEP: Duration = Duration::from_millis(100);
const NO_DATA_AFTER: Duration = Duration::from_secs(3);
const RECONNECT_AFTER_SILENCE: Duration = Duration::from_secs(10);
const RETRY_AFTER_SILENCE: Duration = Duration::from_secs(60);

pub type Quat = [f64; 4];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SensorState {
    #[default]
    Searching,
    NoAccess,
    NoData,
    Working,
}

#[derive(Debug, PartialEq)]
enum ReportsEnded {
    Stopped,
    DeviceClosed,
    Silent { had_data: bool },
}

#[derive(Default)]
pub struct Shared {
    pub rotation: Mutex<Option<Quat>>,
    pub recentre: AtomicBool,
    pub state: Mutex<SensorState>,
    pub stop: AtomicBool,
}

impl Shared {
    pub fn set_state(&self, state: SensorState) {
        *self.state.lock().unwrap() = state;
    }

    fn pause(&self, length: Duration) {
        let started = Instant::now();
        while started.elapsed() < length && !self.stop.load(Ordering::Relaxed) {
            thread::sleep(PAUSE_STEP);
        }
    }
}

pub fn find_device() -> Option<String> {
    let entries = fs::read_dir("/sys/class/hidraw").ok()?;
    for entry in entries.flatten() {
        let descriptor = fs::read(entry.path().join("device/report_descriptor")).unwrap_or_default();
        if descriptor.starts_with(&TRACKER_DESCRIPTOR) {
            return Some(format!("/dev/{}", entry.file_name().to_string_lossy()));
        }
    }
    None
}

fn request(number: u64, length: u64) -> u64 {
    (3 << 30) | (length << 16) | ((b'H' as u64) << 8) | number
}

fn set_sensor(file: &File, state: u8) -> Result<()> {
    let mut report = [0x01, state];
    let result = unsafe { libc::ioctl(file.as_raw_fd(), request(0x06, 2) as _, report.as_mut_ptr()) };
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("could not switch the sensor");
    }
    Ok(())
}

fn open(path: &str) -> std::io::Result<File> {
    OpenOptions::new().read(true).write(true).open(path)
}

fn wait_readable(file: &File, timeout_ms: i32) -> bool {
    let mut poll = libc::pollfd { fd: file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    unsafe { libc::poll(&mut poll, 1, timeout_ms) > 0 }
}

fn multiply(a: Quat, b: Quat) -> Quat {
    [
        a[0] * b[0] - a[1] * b[1] - a[2] * b[2] - a[3] * b[3],
        a[0] * b[1] + a[1] * b[0] + a[2] * b[3] - a[3] * b[2],
        a[0] * b[2] - a[1] * b[3] + a[2] * b[0] + a[3] * b[1],
        a[0] * b[3] + a[1] * b[2] - a[2] * b[1] + a[3] * b[0],
    ]
}

fn orientation(report: &[u8]) -> Quat {
    let raw = |i: usize| i16::from_le_bytes([report[i], report[i + 1]]) as f64 / 32767.0 * std::f64::consts::PI;
    let (x, y, z) = (raw(1), raw(3), raw(5));
    let angle = (x * x + y * y + z * z).sqrt();
    if angle < 1e-12 {
        return [1.0, 0.0, 0.0, 0.0];
    }
    let s = (angle / 2.0).sin() / angle;
    [(angle / 2.0).cos(), x * s, y * s, z * s]
}

/// Azimuth and elevation, in degrees, of a source fixed in the room at `azimuth`, as heard with
/// the head at `rotation`. The head axes are X right, Y forward and Z up. Azimuth grows to the left.
pub fn direction(rotation: Quat, azimuth: f64) -> (f64, f64) {
    let (sin, cos) = azimuth.to_radians().sin_cos();
    let inverse = [rotation[0], -rotation[1], -rotation[2], -rotation[3]];
    let rotated = multiply(multiply(inverse, [0.0, -sin, cos, 0.0]), rotation);
    let (x, y, z) = (rotated[1], rotated[2], rotated[3]);
    ((-x).atan2(y).to_degrees(), z.atan2(x.hypot(y)).to_degrees())
}

/// Smallest angle in degrees that turns `a` into `b`.
pub fn angle_between(a: Quat, b: Quat) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    2.0 * dot.abs().min(1.0).acos().to_degrees()
}

fn read_reports(mut file: &File, shared: &Shared, no_data_after: Duration, give_up_after: Duration) -> ReportsEnded {
    let mut reference: Option<Quat> = None;
    let mut report = [0u8; 64];
    let mut last_report = Instant::now();
    let mut had_data = false;
    while !shared.stop.load(Ordering::Relaxed) {
        if last_report.elapsed() >= give_up_after {
            return ReportsEnded::Silent { had_data };
        }
        if last_report.elapsed() >= no_data_after {
            shared.set_state(SensorState::NoData);
        }
        if !wait_readable(file, 200) {
            continue;
        }
        let length = match file.read(&mut report) {
            Ok(n) if n > 0 => n,
            _ => return ReportsEnded::DeviceClosed,
        };
        if length < 14 || report[0] != 0x01 {
            continue;
        }
        last_report = Instant::now();
        had_data = true;
        shared.set_state(SensorState::Working);
        let current = orientation(&report);
        if shared.recentre.swap(false, Ordering::Relaxed) {
            reference = None;
        }
        let base = *reference.get_or_insert([current[0], -current[1], -current[2], -current[3]]);
        *shared.rotation.lock().unwrap() = Some(multiply(base, current));
    }
    ReportsEnded::Stopped
}

pub fn spawn(shared: Arc<Shared>, address: String) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut silence_limit = RECONNECT_AFTER_SILENCE;
        while !shared.stop.load(Ordering::Relaxed) {
            let Some(path) = find_device() else {
                shared.set_state(SensorState::Searching);
                bluetooth::connect_tracker(&address);
                shared.pause(RECONNECT_INTERVAL);
                continue;
            };
            let file = match open(&path) {
                Ok(file) => file,
                Err(error) => {
                    shared.set_state(if error.kind() == ErrorKind::PermissionDenied { SensorState::NoAccess } else { SensorState::Searching });
                    shared.pause(RECONNECT_INTERVAL);
                    continue;
                }
            };
            if set_sensor(&file, SENSOR_ON).is_ok() {
                shared.set_state(SensorState::Searching);
                let ended = read_reports(&file, &shared, NO_DATA_AFTER, silence_limit);
                let _ = set_sensor(&file, SENSOR_OFF);
                if let ReportsEnded::Silent { had_data } = ended {
                    bluetooth::reconnect_tracker(&address);
                    silence_limit = if had_data { RECONNECT_AFTER_SILENCE } else { RETRY_AFTER_SILENCE };
                    shared.pause(RECONNECT_INTERVAL);
                }
                shared.set_state(SensorState::Searching);
            } else {
                shared.set_state(SensorState::NoData);
                shared.pause(RECONNECT_INTERVAL);
            }
        }
    })
}

/// Prints whether the connected headphones provide a working head tracker.
pub fn check() -> Result<()> {
    let address = bluetooth::find_headphones().ok_or_else(|| anyhow!("no connected Sony headphones with a head tracker service found"))?;
    if find_device().is_none() {
        bluetooth::connect_tracker(&address);
        thread::sleep(Duration::from_secs(2));
    }
    let path = find_device().ok_or_else(|| anyhow!("the headphones did not provide a head tracker device"))?;
    let name = fs::read_to_string(format!("/sys/class/hidraw/{}/device/uevent", path.trim_start_matches("/dev/")))?;
    for line in name.lines().filter(|l| l.starts_with("HID_NAME") || l.starts_with("HID_ID")) {
        println!("{line}");
    }
    let file = open(&path).with_context(|| format!("could not open {path}, is the access rule installed?"))?;
    let mut report = [0u8; 24];
    report[0] = 0x02;
    let result = unsafe { libc::ioctl(file.as_raw_fd(), request(0x07, 24) as _, report.as_mut_ptr()) };
    println!("android head tracker: {}", if result >= 0 && report[1..].starts_with(MARKER) { "yes" } else { "no" });
    set_sensor(&file, SENSOR_ON)?;
    let mut reports = 0;
    let mut buffer = [0u8; 64];
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        if wait_readable(&file, 100) && (&file).read(&mut buffer)? >= 14 {
            reports += 1;
        }
    }
    set_sensor(&file, SENSOR_OFF)?;
    println!("sensor updates per second: {:.1}", reports as f64 / 2.0);
    println!("result: {}", if reports > 0 { "working" } else { "no sensor data" });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn around(axis: [f64; 3], degrees: f64) -> Quat {
        let (sin, cos) = (degrees.to_radians() / 2.0).sin_cos();
        [cos, axis[0] * sin, axis[1] * sin, axis[2] * sin]
    }

    #[test]
    fn direction_follows_head_rotation() {
        let right = [1.0, 0.0, 0.0];
        let forward = [0.0, 1.0, 0.0];
        let up = [0.0, 0.0, 1.0];
        let cases = [
            ("no rotation", around(up, 0.0), 45.0, (45.0, 0.0)),
            ("turn left", around(up, 30.0), 0.0, (-30.0, 0.0)),
            ("turn right", around(up, -30.0), 45.0, (75.0, 0.0)),
            ("look up", around(right, 30.0), 0.0, (0.0, -30.0)),
            ("look down", around(right, -30.0), 0.0, (0.0, 30.0)),
            ("tilt right", around(forward, 30.0), 90.0, (90.0, -30.0)),
        ];
        for (name, rotation, azimuth, expected) in cases {
            let (found_azimuth, found_elevation) = direction(rotation, azimuth);
            assert!(
                (found_azimuth - expected.0).abs() < 1e-9 && (found_elevation - expected.1).abs() < 1e-9,
                "{name}: got ({found_azimuth}, {found_elevation}), expected {expected:?}"
            );
        }
    }

    fn pipe() -> (File, File) {
        use std::os::fd::FromRawFd;
        let mut ends = [0; 2];
        assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
        unsafe { (File::from_raw_fd(ends[0]), File::from_raw_fd(ends[1])) }
    }

    #[test]
    fn read_reports_follows_whether_data_arrives() {
        use std::io::Write;

        fn wait_for(shared: &Shared, expected: SensorState) {
            let started = Instant::now();
            while *shared.state.lock().unwrap() != expected {
                assert!(started.elapsed() < Duration::from_secs(5), "never reached {expected:?}");
                thread::sleep(Duration::from_millis(5));
            }
        }

        let (reader, mut writer) = pipe();
        let shared = Arc::new(Shared::default());
        let worker = {
            let shared = shared.clone();
            thread::spawn(move || read_reports(&reader, &shared, Duration::from_millis(50), Duration::from_secs(60)))
        };
        wait_for(&shared, SensorState::NoData);
        writer.write_all(&[1; 14]).unwrap();
        wait_for(&shared, SensorState::Working);
        wait_for(&shared, SensorState::NoData);
        shared.stop.store(true, Ordering::Relaxed);
        assert_eq!(worker.join().unwrap(), ReportsEnded::Stopped);
    }

    #[test]
    fn read_reports_gives_up_after_long_silence() {
        use std::io::Write;

        for (send_report, expected) in [(false, ReportsEnded::Silent { had_data: false }), (true, ReportsEnded::Silent { had_data: true })] {
            let (reader, mut writer) = pipe();
            if send_report {
                writer.write_all(&[1; 14]).unwrap();
            }
            let ended = read_reports(&reader, &Shared::default(), Duration::from_millis(10), Duration::from_millis(50));
            assert_eq!(ended, expected);
        }
    }

    #[test]
    fn angle_between_ignores_quaternion_sign() {
        let a = around([0.0, 0.0, 1.0], 10.0);
        let negated = [-a[0], -a[1], -a[2], -a[3]];
        assert!(angle_between(a, negated).abs() < 1e-6);
        assert!((angle_between(around([0.0, 0.0, 1.0], 0.0), a) - 10.0).abs() < 1e-6);
    }
}
