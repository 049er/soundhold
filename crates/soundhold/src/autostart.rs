use crate::settings::user_folder;
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Command;

const UNIT: &str = "soundhold.service";
const TEMPLATE: &str = "[Unit]
Description=Soundhold head tracked spatial sound
After=graphical-session.target pipewire.service wireplumber.service
PartOf=graphical-session.target

[Service]
ExecStart=@PROGRAM@ daemon
Restart=on-failure

[Install]
WantedBy=graphical-session.target
";

fn systemctl(args: &[&str]) -> Result<std::process::Output> {
    Command::new("systemctl").arg("--user").args(args).output().context("could not run systemctl")
}

fn unit_path() -> Result<PathBuf> {
    Ok(user_folder("XDG_CONFIG_HOME", ".config")?.join("systemd/user").join(UNIT))
}

pub fn enabled() -> bool {
    systemctl(&["is-enabled", UNIT]).is_ok_and(|output| output.status.success())
}

pub fn set(enable: bool) -> Result<()> {
    let path = unit_path()?;
    if enable {
        let program = std::env::current_exe()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, TEMPLATE.replace("@PROGRAM@", &program.to_string_lossy()))?;
        systemctl(&["daemon-reload"])?;
    }
    let output = systemctl(&[if enable { "enable" } else { "disable" }, UNIT])?;
    if !output.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    if !enable {
        std::fs::remove_file(&path)?;
        systemctl(&["daemon-reload"])?;
    }
    Ok(())
}
