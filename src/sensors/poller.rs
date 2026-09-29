use super::{
    libre_hardware_monitor::Reader as HardwareReader,
    model::{timestamp_ms, NamedSensor, SensorInfo, SensorSnapshot},
    weather,
    windows_volume::Reader as VolumeReader,
};
use crate::config::schema::{CpuClockSource, CpuTemperatureSource, WeatherConfig};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use sysinfo::{CpuRefreshKind, Disk, Disks, MemoryRefreshKind, Networks, RefreshKind, System};

pub struct SensorPoller {
    system: System,
    networks: Networks,
    disks: Disks,
    hardware_reader: Option<HardwareReader>,
    hardware_path: Option<PathBuf>,
    hardware_snapshot: SensorSnapshot,
    last_network_refresh: Instant,
    last_hardware_refresh: Option<Instant>,
    hardware_refresh_interval: Duration,
    volume_reader: VolumeReader,
    weather_snapshot: SensorSnapshot,
    last_weather_refresh: Option<Instant>,
    hardware_sampled_at: Option<u64>,
    weather_sampled_at: Option<u64>,
    hardware_error: Option<String>,
    weather_error: Option<String>,
    weather_config: Option<WeatherConfig>,
}

impl SensorPoller {
    pub fn new() -> Self {
        let mut system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        system.refresh_cpu_usage();
        system.refresh_memory();
        let networks = Networks::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();
        Self {
            system,
            networks,
            disks,
            hardware_reader: None,
            hardware_path: None,
            hardware_snapshot: SensorSnapshot::default(),
            last_network_refresh: Instant::now(),
            last_hardware_refresh: None,
            hardware_refresh_interval: Duration::from_secs(2),
            volume_reader: VolumeReader::new(),
            weather_snapshot: SensorSnapshot::default(),
            last_weather_refresh: None,
            hardware_sampled_at: None,
            weather_sampled_at: None,
            hardware_error: None,
            weather_error: None,
            weather_config: None,
        }
    }

    pub fn set_hardware_refresh_interval(&mut self, interval: Duration) {
        self.hardware_refresh_interval = interval.max(Duration::from_millis(250));
    }

    pub fn read(
        &mut self,
        lhm_dll: Option<&str>,
        cpu_temperature_source: CpuTemperatureSource,
        cpu_clock_source: CpuClockSource,
        fan_sensor: Option<&str>,
        disk_source: Option<&str>,
        weather_config: &WeatherConfig,
    ) -> SensorSnapshot {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.networks.refresh(true);
        self.disks.refresh(false);
        let system_sampled_at = timestamp_ms();
        let network_interval = self.last_network_refresh.elapsed().as_secs_f32().max(0.001);
        self.last_network_refresh = Instant::now();

        let selected_disk = select_disk(&self.disks, disk_source);
        let disk_usage = selected_disk.and_then(disk_usage_percent);
        let actual_disk = selected_disk
            .map(disk_label)
            .unwrap_or_else(|| "unavailable".into());
        let disk_sensors = disk_sensors(&self.disks);
        let network_download = self
            .networks
            .values()
            .map(|network| network.received())
            .sum::<u64>() as f32
            / 1024.0
            / network_interval;
        let network_upload = self
            .networks
            .values()
            .map(|network| network.transmitted())
            .sum::<u64>() as f32
            / 1024.0
            / network_interval;

        let hardware_due = self
            .last_hardware_refresh
            .is_none_or(|last| last.elapsed() >= self.hardware_refresh_interval);
        if hardware_due {
            self.refresh_hardware(lhm_dll);
            self.last_hardware_refresh = Some(Instant::now());
        }
        let hardware = &self.hardware_snapshot;
        if self.weather_config.as_ref() != Some(weather_config) {
            self.weather_config = Some(weather_config.clone());
            self.weather_snapshot = SensorSnapshot::default();
            self.weather_sampled_at = None;
            self.last_weather_refresh = None;
            self.weather_error = None;
        }
        let weather_due = weather_config.enabled
            && self.last_weather_refresh.is_none_or(|last| {
                last.elapsed() >= Duration::from_secs(weather_config.refresh_minutes.max(5) * 60)
            });
        if weather_due {
            match weather::read(weather_config) {
                Ok(snapshot) => {
                    self.weather_snapshot = snapshot;
                    self.weather_sampled_at = Some(timestamp_ms());
                    self.weather_error = None;
                }
                Err(error) => {
                    self.weather_error = Some(format!("{error:#}"));
                    tracing::warn!(error = %format!("{error:#}"), "weather refresh failed")
                }
            }
            self.last_weather_refresh = Some(Instant::now());
        }
        let disabled_weather = SensorSnapshot::default();
        let weather = if weather_config.enabled {
            &self.weather_snapshot
        } else {
            &disabled_weather
        };
        let mut snapshot = SensorSnapshot {
            cpu_temperature: select_cpu_temperature(hardware, cpu_temperature_source),
            cpu_temperature_core: hardware.cpu_temperature_core,
            cpu_temperature_socket: hardware.cpu_temperature_socket,
            cpu_usage: Some(self.system.global_cpu_usage()),
            cpu_clock: select_cpu_clock(hardware, cpu_clock_source),
            cpu_clock_average: hardware.cpu_clock_average,
            cpu_clock_effective: hardware.cpu_clock_effective,
            gpu_temperature: hardware.gpu_temperature,
            gpu_usage: hardware.gpu_usage,
            gpu_clock: hardware.gpu_clock,
            gpu_power: hardware.gpu_power,
            ram_usage: if self.system.total_memory() > 0 {
                Some(self.system.used_memory() as f32 / self.system.total_memory() as f32 * 100.0)
            } else {
                None
            },
            vram_usage: hardware.vram_usage,
            vram_used_mb: hardware.vram_used_mb,
            vram_total_mb: hardware.vram_total_mb,
            disk_usage,
            network_upload: Some(network_upload),
            network_download: Some(network_download),
            fan_speed: fan_sensor
                .and_then(|id| {
                    hardware
                        .fan_sensors
                        .iter()
                        .find(|sensor| sensor.id == id)
                        .map(|sensor| sensor.value)
                })
                .or(hardware.fan_speed),
            system_volume: self.volume_reader.read(),
            weather_temperature: weather.weather_temperature,
            weather_humidity: weather.weather_humidity,
            weather_wind_speed: weather.weather_wind_speed,
            weather_code: weather.weather_code,
            weather_condition: weather.weather_condition.clone(),
            fan_sensors: hardware.fan_sensors.clone(),
            disk_sensors,
            ..Default::default()
        };
        let cpu_temperature_source = match cpu_temperature_source {
            CpuTemperatureSource::Socket if hardware.cpu_temperature_socket.is_some() => "socket",
            _ if hardware.cpu_temperature_core.is_some() => "core/package",
            _ if hardware.cpu_temperature_socket.is_some() => "socket (fallback)",
            _ => "automatic",
        };
        let cpu_clock_source = match cpu_clock_source {
            CpuClockSource::Effective if hardware.cpu_clock_effective.is_some() => "effective",
            _ if hardware.cpu_clock_average.is_some() => "average",
            _ => "effective (fallback)",
        };
        let actual_fan = fan_sensor
            .and_then(|id| hardware.fan_sensors.iter().find(|sensor| sensor.id == id))
            .map(|sensor| format!("{} ({})", sensor.name, sensor.id))
            .unwrap_or_else(|| "automatic".into());
        let hardware_stale = (self.hardware_refresh_interval.as_millis() as u64 * 3).max(10_000);
        for (key, value, unit, source, hardware_sensor) in [
            (
                "CPU usage",
                snapshot.cpu_usage,
                "%",
                "Windows / sysinfo".to_string(),
                false,
            ),
            (
                "CPU temperature",
                snapshot.cpu_temperature,
                "°C",
                format!("LibreHardwareMonitor / {cpu_temperature_source}"),
                true,
            ),
            (
                "CPU clock",
                snapshot.cpu_clock,
                "MHz",
                format!("LibreHardwareMonitor / {cpu_clock_source}"),
                true,
            ),
            (
                "GPU usage",
                snapshot.gpu_usage,
                "%",
                "LibreHardwareMonitor".into(),
                true,
            ),
            (
                "GPU temperature",
                snapshot.gpu_temperature,
                "°C",
                "LibreHardwareMonitor".into(),
                true,
            ),
            (
                "GPU clock",
                snapshot.gpu_clock,
                "MHz",
                "LibreHardwareMonitor".into(),
                true,
            ),
            (
                "GPU power",
                snapshot.gpu_power,
                "W",
                "LibreHardwareMonitor".into(),
                true,
            ),
            (
                "RAM",
                snapshot.ram_usage,
                "%",
                "Windows / sysinfo".into(),
                false,
            ),
            (
                "VRAM",
                snapshot.vram_usage,
                "%",
                "LibreHardwareMonitor".into(),
                true,
            ),
            (
                "Disk",
                snapshot.disk_usage,
                "%",
                format!("Windows / sysinfo / {actual_disk}"),
                false,
            ),
            (
                "Fan",
                snapshot.fan_speed,
                "rpm",
                format!("LibreHardwareMonitor / {actual_fan}"),
                true,
            ),
            (
                "Download",
                snapshot.network_download,
                "KB/s",
                "Windows / sysinfo".into(),
                false,
            ),
            (
                "Upload",
                snapshot.network_upload,
                "KB/s",
                "Windows / sysinfo".into(),
                false,
            ),
            (
                "Volume",
                snapshot.system_volume,
                "%",
                "Windows Core Audio".into(),
                false,
            ),
        ] {
            snapshot.sensor_info.push(SensorInfo {
                key: key.into(),
                value,
                unit: unit.into(),
                source,
                sampled_at_ms: if hardware_sensor {
                    self.hardware_sampled_at
                } else {
                    Some(system_sampled_at)
                },
                stale_after_ms: if hardware_sensor {
                    hardware_stale
                } else {
                    10_000
                },
                error: if hardware_sensor {
                    self.hardware_error.clone()
                } else {
                    None
                },
            });
        }
        snapshot.sensor_info.push(SensorInfo {
            key: "Weather".into(),
            value: snapshot.weather_temperature,
            unit: "°C".into(),
            source: "Open-Meteo".into(),
            sampled_at_ms: if weather_config.enabled {
                self.weather_sampled_at
            } else {
                None
            },
            stale_after_ms: weather_config
                .refresh_minutes
                .max(5)
                .saturating_mul(120_000),
            error: if weather_config.enabled {
                self.weather_error.clone()
            } else {
                Some("Weather is disabled".into())
            },
        });
        snapshot
    }

    fn refresh_hardware(&mut self, configured_path: Option<&str>) {
        let path = resolve_lhm_path(configured_path);
        if path != self.hardware_path {
            self.hardware_reader = None;
            self.hardware_snapshot = SensorSnapshot::default();
            self.hardware_path = path.clone();
            self.hardware_sampled_at = None;
        }
        let Some(path) = path else {
            self.hardware_error = Some("LibreHardwareMonitor DLL not found".into());
            return;
        };
        if self.hardware_reader.is_none() {
            match HardwareReader::start(&path) {
                Ok(reader) => self.hardware_reader = Some(reader),
                Err(error) => {
                    self.hardware_error = Some(format!("{error:#}"));
                    tracing::error!(
                        dll = %path.display(),
                        error = %format!("{error:#}"),
                        "LibreHardwareMonitor bridge startup failed"
                    );
                    return;
                }
            }
        }
        let result = self
            .hardware_reader
            .as_mut()
            .expect("hardware reader initialized")
            .read();
        match result {
            Ok(snapshot) => {
                self.hardware_snapshot = snapshot;
                self.hardware_sampled_at = Some(timestamp_ms());
                self.hardware_error = None;
            }
            Err(error) => {
                self.hardware_error = Some(format!("{error:#}"));
                tracing::error!(
                    dll = %path.display(),
                    error = %format!("{error:#}"),
                    "LibreHardwareMonitor read failed"
                );
                self.hardware_reader = None;
                self.hardware_snapshot = SensorSnapshot::default();
            }
        }
    }
}

impl Default for SensorPoller {
    fn default() -> Self {
        Self::new()
    }
}

pub fn read_snapshot(
    lhm_dll: Option<&str>,
    cpu_temperature_source: CpuTemperatureSource,
    cpu_clock_source: CpuClockSource,
    fan_sensor: Option<&str>,
    disk_source: Option<&str>,
    weather_config: &WeatherConfig,
) -> SensorSnapshot {
    let mut poller = SensorPoller::new();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    poller.read(
        lhm_dll,
        cpu_temperature_source,
        cpu_clock_source,
        fan_sensor,
        disk_source,
        weather_config,
    )
}

fn disk_id(disk: &Disk) -> String {
    let mount_point = disk.mount_point().to_string_lossy();
    if !mount_point.is_empty() {
        mount_point.into_owned()
    } else {
        disk.name().to_string_lossy().into_owned()
    }
}

fn disk_label(disk: &Disk) -> String {
    let id = disk_id(disk);
    let name = disk.name().to_string_lossy();
    let size_gb = disk.total_space() as f32 / 1_073_741_824.0;
    if name.is_empty() || name == id {
        format!("{id} ({size_gb:.0} GB)")
    } else {
        format!("{id} {name} ({size_gb:.0} GB)")
    }
}

fn disk_usage_percent(disk: &Disk) -> Option<f32> {
    (disk.total_space() > 0).then(|| {
        (disk.total_space() - disk.available_space()) as f32 / disk.total_space() as f32 * 100.0
    })
}

fn disk_sensors(disks: &Disks) -> Vec<NamedSensor> {
    disks
        .list()
        .iter()
        .filter_map(|disk| {
            disk_usage_percent(disk).map(|value| NamedSensor {
                id: disk_id(disk),
                name: disk_label(disk),
                value,
            })
        })
        .collect()
}

fn select_disk<'a>(disks: &'a Disks, source: Option<&str>) -> Option<&'a Disk> {
    let valid_disks = || disks.list().iter().filter(|disk| disk.total_space() > 0);
    source
        .and_then(|id| valid_disks().find(|disk| disk_id(disk) == id))
        .or_else(|| valid_disks().max_by_key(|disk| disk.total_space()))
}

fn resolve_lhm_path(configured_path: Option<&str>) -> Option<PathBuf> {
    configured_path
        .filter(|path| Path::new(path).exists())
        .map(PathBuf::from)
        .or_else(|| {
            [
                r"C:\Program Files\LibreHardwareMonitor\LibreHardwareMonitorLib.dll",
                r"C:\Program Files (x86)\LibreHardwareMonitor\LibreHardwareMonitorLib.dll",
                r"C:\Program Files\FanControl\LibreHardwareMonitorLib.dll",
                r"C:\Program Files (x86)\FanControl\LibreHardwareMonitorLib.dll",
            ]
            .into_iter()
            .find(|path| Path::new(path).exists())
            .map(PathBuf::from)
        })
}

fn select_cpu_temperature(snapshot: &SensorSnapshot, source: CpuTemperatureSource) -> Option<f32> {
    match source {
        CpuTemperatureSource::Socket => snapshot
            .cpu_temperature_socket
            .or(snapshot.cpu_temperature_core),
        CpuTemperatureSource::Auto | CpuTemperatureSource::Core => snapshot
            .cpu_temperature_core
            .or(snapshot.cpu_temperature_socket)
            .or(snapshot.cpu_temperature),
    }
}

fn select_cpu_clock(snapshot: &SensorSnapshot, source: CpuClockSource) -> Option<f32> {
    match source {
        CpuClockSource::Average => snapshot.cpu_clock_average.or(snapshot.cpu_clock_effective),
        CpuClockSource::Effective => snapshot.cpu_clock_effective.or(snapshot.cpu_clock_average),
    }
}
