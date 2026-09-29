use super::model::{NamedSensor, SensorSnapshot};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::{
    io::{BufRead, BufReader, Write},
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::Duration,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Deserialize)]
struct LhmOutput {
    cpu_temperature: Option<f32>,
    cpu_temperature_core: Option<f32>,
    cpu_temperature_socket: Option<f32>,
    cpu_clock_average: Option<f32>,
    cpu_clock_effective: Option<f32>,
    gpu_temperature: Option<f32>,
    gpu_usage: Option<f32>,
    gpu_clock: Option<f32>,
    gpu_power: Option<f32>,
    vram_usage: Option<f32>,
    vram_used_mb: Option<f32>,
    vram_total_mb: Option<f32>,
    fan_speed: Option<f32>,
    #[serde(default)]
    fan_sensors: Vec<NamedSensor>,
}

pub struct Reader {
    child: Child,
    stdin: ChildStdin,
    output: Receiver<std::io::Result<String>>,
    first_read: bool,
}

impl Reader {
    pub fn start(dll: &Path) -> Result<Self> {
        let script = include_str!("../../scripts/read-lhm.ps1");
        let shell = powershell();
        let mut command = Command::new(shell);
        command
            .creation_flags(CREATE_NO_WINDOW)
            .env("TURZX_LHM_DLL", dll)
            .env("TELEMETRYFORGE_PERSISTENT", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                script,
            ]);
        let mut child = command
            .spawn()
            .context("Could not start the LibreHardwareMonitor bridge")?;
        let stdin = child.stdin.take().context("Bridge stdin is unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("Bridge stdout is unavailable")?;
        let (send, output) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                let result = reader.read_line(&mut line).and_then(|size| {
                    if size == 0 {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "Hardware bridge exited",
                        ))
                    } else {
                        Ok(line)
                    }
                });
                let failed = result.is_err();
                if send.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            output,
            first_read: true,
        })
    }

    pub fn read(&mut self) -> Result<SensorSnapshot> {
        self.stdin.write_all(b"read\n")?;
        self.stdin.flush()?;
        let timeout = if self.first_read { 20 } else { 5 };
        self.first_read = false;
        let line = self
            .output
            .recv_timeout(Duration::from_secs(timeout))
            .context("LibreHardwareMonitor bridge timed out or disconnected")??;
        parse(&line)
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"quit\n");
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn powershell() -> &'static str {
    let mut probe = Command::new("pwsh.exe");
    probe.creation_flags(CREATE_NO_WINDOW).args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "exit 0",
    ]);
    if probe.status().is_ok() {
        "pwsh.exe"
    } else {
        "powershell.exe"
    }
}

fn parse(json: &str) -> Result<SensorSnapshot> {
    let parsed: LhmOutput = serde_json::from_str(json.trim())?;
    Ok(SensorSnapshot {
        cpu_temperature: parsed.cpu_temperature,
        cpu_temperature_core: parsed.cpu_temperature_core,
        cpu_temperature_socket: parsed.cpu_temperature_socket,
        cpu_clock_average: parsed.cpu_clock_average,
        cpu_clock_effective: parsed.cpu_clock_effective,
        gpu_temperature: parsed.gpu_temperature,
        gpu_usage: parsed.gpu_usage,
        gpu_clock: parsed.gpu_clock,
        gpu_power: parsed.gpu_power,
        vram_usage: parsed.vram_usage,
        vram_used_mb: parsed.vram_used_mb,
        vram_total_mb: parsed.vram_total_mb,
        fan_speed: parsed.fan_speed,
        fan_sensors: parsed.fan_sensors,
        ..Default::default()
    })
}
