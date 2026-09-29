//! Manual sample authoring recipe; never runs during app startup or CI tests.
use crate::{
    config::{persistence, schema::*},
    renderer,
    sensors::model::SensorSnapshot,
};
use std::{collections::HashMap, fs};

const NAME: &str = "telemetry";
const WHITE: &str = "#edf4f6";
const MUTED: &str = "#9babb9";
const CYAN: &str = "#50e2cd";
const ORANGE: &str = "#ffb365";

#[test]
#[ignore = "regenerates the public portable telemetry sample without personal settings or artwork"]
fn export_public_telemetry_sample() {
    let mut sample = design(AppConfig::default());
    sample.background = BackgroundConfig::default();
    sample.background.colour = "#101a29".into();
    sample.weather = WeatherConfig::default();
    sample.theme.name = "Telemetry".into();
    if let Some(caption) = sample
        .widgets
        .iter_mut()
        .find(|widget| widget.editor_name == "Edition")
    {
        caption.label_format = "F O R G E  /  0 1".into();
    }
    sample.widgets.push(text(
        "Monogram",
        WidgetKind::Text,
        190,
        115,
        110,
        64.0,
        "TF",
        WHITE,
        "04 • Artwork",
    ));
    sample.widgets.push(text(
        "Signature",
        WidgetKind::Text,
        178,
        179,
        140,
        12.0,
        "S Y S T E M  /  L I V E",
        MUTED,
        "04 • Artwork",
    ));
    let output = std::path::Path::new("samples/telemetry");
    fs::create_dir_all(output).unwrap();
    fs::write(
        output.join("config.yaml"),
        serde_yaml::to_string(&sample).unwrap(),
    )
    .unwrap();
    crate::package::export(
        std::path::Path::new("samples/telemetry.telemetryforge"),
        &sample,
    )
    .unwrap();
    let sensors = SensorSnapshot {
        cpu_usage: Some(38.0),
        cpu_temperature: Some(57.0),
        cpu_clock: Some(4850.0),
        gpu_usage: Some(76.0),
        gpu_temperature: Some(64.0),
        gpu_clock: Some(2670.0),
        gpu_power: Some(187.0),
        ram_usage: Some(42.0),
        vram_usage: Some(61.0),
        disk_usage: Some(68.0),
        fan_speed: Some(1140.0),
        network_download: Some(286.0),
        network_upload: Some(24.0),
        system_volume: Some(36.0),
        weather_temperature: Some(18.0),
        weather_code: Some(2),
        history_cpu: (0..80)
            .map(|i| 25.0 + (i as f32 * 0.4).sin() * 13.0)
            .collect(),
        history_gpu: (0..80)
            .map(|i| 60.0 + (i as f32 * 0.23).sin() * 20.0)
            .collect(),
        ..Default::default()
    };
    renderer::render(&sample, &sensors, &HashMap::new())
        .unwrap()
        .save(output.join("preview.png"))
        .unwrap();
}

fn text(
    name: &str,
    kind: WidgetKind,
    x: i32,
    y: i32,
    width: u32,
    size: f32,
    label: &str,
    colour: &str,
    group: &str,
) -> WidgetConfig {
    let mut w = WidgetConfig::new(kind, x, y, label);
    w.editor_name = name.into();
    w.group_id = Some(group.into());
    w.width = width;
    w.height = size.ceil() as u32 + 5;
    w.font = "Bahnschrift".into();
    w.font_size = size;
    w.colour = colour.into();
    w.refresh_interval_ms = if kind == WidgetKind::Volume {
        100
    } else {
        1000
    };
    w
}

fn panel(
    name: &str,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    colour: &str,
    alpha: f32,
) -> WidgetConfig {
    let mut w = WidgetConfig::new(WidgetKind::Text, x, y, "");
    w.editor_name = name.into();
    w.group_id = Some("00 • Structure".into());
    w.locked = true;
    w.width = width;
    w.height = height;
    w.render_mode = WidgetRenderMode::Graph;
    w.opacity = 0.0;
    w.graph_background_colour = colour.into();
    w.graph_background_opacity = alpha;
    w
}

fn indicator(
    kind: WidgetKind,
    mode: WidgetRenderMode,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    colour: &str,
    group: &str,
) -> WidgetConfig {
    let mut w = text(
        "Indicator",
        kind,
        x,
        y,
        width,
        16.0,
        "{value}",
        colour,
        group,
    );
    w.render_mode = mode;
    w.height = height;
    w.secondary_colour = colour.into();
    w.circle_start_angle = 135.0;
    w.circle_sweep_angle = 270.0;
    w.circle_thickness = 5.0;
    if mode == WidgetRenderMode::Circle {
        w.glow = 3;
    }
    w.use_thresholds = true;
    w.warning_threshold = 75.0;
    w.critical_threshold = 92.0;
    w.warning_colour = "#edc466".into();
    w.critical_colour = "#ff5b64".into();
    w
}

fn design(mut config: AppConfig) -> AppConfig {
    config.display.width = 480;
    config.display.height = 320;
    config.theme.name = "Telemetry / Stellar Edition".into();
    config.theme.foreground = WHITE.into();
    config.theme.accent = CYAN.into();
    config.theme.panel = "#0c141e".into();
    config.widgets.clear();
    let ws = &mut config.widgets;
    ws.push(panel("Top / smoked glass", 8, 8, 464, 30, "#080e15", 0.92));
    ws.push(panel(
        "CPU / smoked glass",
        8,
        46,
        144,
        184,
        "#080e15",
        0.91,
    ));
    ws.push(panel(
        "GPU / smoked glass",
        328,
        46,
        144,
        184,
        "#080e15",
        0.91,
    ));
    ws.push(panel(
        "Resources / smoked glass",
        8,
        238,
        464,
        74,
        "#080e15",
        0.94,
    ));
    ws.push(panel("CPU accent", 8, 46, 2, 184, CYAN, 1.0));
    ws.push(panel("GPU accent", 470, 46, 2, 184, ORANGE, 1.0));
    ws.push(text(
        "Brand",
        WidgetKind::Text,
        18,
        14,
        150,
        17.0,
        "T E L E M E T R Y",
        WHITE,
        "01 • Header",
    ));
    ws.push(panel("Header / cyan hairline", 8, 37, 144, 1, CYAN, 0.85));
    ws.push(panel(
        "Header / amber hairline",
        328,
        37,
        144,
        1,
        ORANGE,
        0.85,
    ));
    ws.push(text(
        "Weather",
        WidgetKind::WeatherTemperature,
        305,
        15,
        62,
        16.0,
        "{value} °C",
        WHITE,
        "01 • Header",
    ));
    let mut icon = text(
        "Weather icon",
        WidgetKind::WeatherIcon,
        278,
        11,
        22,
        25.0,
        "{value}",
        MUTED,
        "01 • Header",
    );
    icon.font = "Segoe UI Symbol".into();
    icon.height = 23;
    ws.push(icon);
    ws.push(text(
        "Clock",
        WidgetKind::Clock,
        405,
        11,
        62,
        23.0,
        "{value}",
        WHITE,
        "01 • Header",
    ));

    for (x, colour, group, label, usage, temp, clock) in [
        (
            8,
            CYAN,
            "02 • CPU",
            "CPU",
            WidgetKind::CpuUsage,
            WidgetKind::CpuTemperature,
            WidgetKind::CpuClock,
        ),
        (
            328,
            ORANGE,
            "03 • GPU",
            "GPU",
            WidgetKind::GpuUsage,
            WidgetKind::GpuTemperature,
            WidgetKind::GpuClock,
        ),
    ] {
        ws.push(text(
            &format!("{label} / title"),
            WidgetKind::Text,
            x + 12,
            53,
            50,
            18.0,
            label,
            colour,
            group,
        ));
        ws.push(text(
            &format!("{label} / frequency"),
            clock,
            x + 63,
            56,
            78,
            14.0,
            "{value}",
            MUTED,
            group,
        ));
        let mut dial = indicator(
            usage,
            WidgetRenderMode::Circle,
            x + 30,
            81,
            84,
            84,
            colour,
            group,
        );
        dial.editor_name = format!("{label} / load arc");
        ws.push(dial);
        ws.push(text(
            &format!("{label} / load value"),
            usage,
            x + 47,
            103,
            68,
            32.0,
            "{value}",
            WHITE,
            group,
        ));
        ws.push(text(
            &format!("{label} / load unit"),
            WidgetKind::Text,
            x + 53,
            136,
            53,
            12.0,
            "LOAD %",
            MUTED,
            group,
        ));
        ws.push(text(
            &format!("{label} / thermal label"),
            WidgetKind::Text,
            x + 12,
            175,
            50,
            12.0,
            "TEMP",
            MUTED,
            group,
        ));
        let mut thermal = text(
            &format!("{label} / temperature"),
            temp,
            x + 65,
            166,
            77,
            27.0,
            "{value}°",
            WHITE,
            group,
        );
        thermal.use_thresholds = true;
        thermal.warning_threshold = 75.0;
        thermal.critical_threshold = 88.0;
        ws.push(thermal);
        let mut line = indicator(
            usage,
            WidgetRenderMode::Graph,
            x + 12,
            204,
            120,
            16,
            colour,
            group,
        );
        line.editor_name = format!("{label} / load history");
        line.opacity = 0.75;
        line.use_thresholds = false;
        ws.push(line);
    }
    // Keep a central uninterrupted window for the source screen's slideshow.
    ws.push(panel(
        "Centre / subtle caption",
        164,
        205,
        152,
        25,
        "#080e15",
        0.7,
    ));
    ws.push(text(
        "Edition",
        WidgetKind::Text,
        178,
        210,
        136,
        12.0,
        "S T E L L A R  /  0 1",
        WHITE,
        "04 • Artwork",
    ));

    for (kind, label, x, width) in [
        (WidgetKind::RamUsage, "RAM", 20, 100),
        (WidgetKind::VramUsage, "VRAM", 135, 100),
        (WidgetKind::DiskUsage, "SSD", 250, 96),
    ] {
        let group = "05 • Resources";
        ws.push(text(
            &format!("{label} / label"),
            WidgetKind::Text,
            x,
            246,
            44,
            13.0,
            label,
            MUTED,
            group,
        ));
        ws.push(text(
            &format!("{label} / value"),
            kind,
            x + 47,
            242,
            60,
            21.0,
            "{value}%",
            WHITE,
            group,
        ));
        let mut bar = indicator(kind, WidgetRenderMode::Bar, x, 271, width, 4, CYAN, group);
        bar.editor_name = format!("{label} / bar");
        ws.push(bar);
    }
    ws.push(text(
        "Audio / label",
        WidgetKind::Text,
        366,
        246,
        35,
        13.0,
        "VOL",
        MUTED,
        "06 • Audio",
    ));
    ws.push(text(
        "Audio / value",
        WidgetKind::Volume,
        405,
        242,
        60,
        21.0,
        "{value}%",
        WHITE,
        "06 • Audio",
    ));
    let mut vol = indicator(
        WidgetKind::Volume,
        WidgetRenderMode::Bar,
        366,
        271,
        94,
        4,
        ORANGE,
        "06 • Audio",
    );
    vol.use_thresholds = false;
    vol.refresh_interval_ms = 100;
    ws.push(vol);
    ws.push(text(
        "Fan speed",
        WidgetKind::FanSpeed,
        20,
        288,
        118,
        14.0,
        "FAN {value} rpm",
        MUTED,
        "07 • Status",
    ));
    ws.push(text(
        "GPU power",
        WidgetKind::GpuPower,
        144,
        288,
        78,
        14.0,
        "{value} W",
        ORANGE,
        "07 • Status",
    ));
    ws.push(text(
        "Download",
        WidgetKind::NetworkDownload,
        241,
        288,
        100,
        14.0,
        "D {value} KB/s",
        MUTED,
        "07 • Status",
    ));
    ws.push(text(
        "Upload",
        WidgetKind::NetworkUpload,
        355,
        288,
        108,
        14.0,
        "U {value} KB/s",
        MUTED,
        "07 • Status",
    ));
    config
}

#[test]
#[ignore = "manually clones the user's test screen and renders design proofs"]
fn create_telemetry_screen() {
    let source_path =
        persistence::profiles_dir(&persistence::default_config_path()).join("test.yaml");
    let original: AppConfig =
        serde_yaml::from_str(&fs::read_to_string(&source_path).unwrap()).unwrap();
    let mut sample = design(original.clone());
    sample.remote = Default::default();
    let destination = persistence::profile_path(&persistence::default_config_path(), NAME).unwrap();
    persistence::save(&destination, &sample).unwrap();
    let sensors = SensorSnapshot {
        cpu_usage: Some(38.0),
        cpu_temperature: Some(57.0),
        cpu_clock: Some(4850.0),
        gpu_usage: Some(76.0),
        gpu_temperature: Some(64.0),
        gpu_clock: Some(2670.0),
        gpu_power: Some(187.0),
        ram_usage: Some(42.0),
        vram_usage: Some(61.0),
        disk_usage: Some(68.0),
        fan_speed: Some(1140.0),
        network_download: Some(286.0),
        network_upload: Some(24.0),
        system_volume: Some(36.0),
        weather_temperature: Some(18.0),
        weather_code: Some(2),
        history_cpu: (0..80)
            .map(|i| 25.0 + ((i as f32) * 0.4).sin() * 13.0)
            .collect(),
        history_gpu: (0..80)
            .map(|i| 60.0 + ((i as f32) * 0.23).sin() * 20.0)
            .collect(),
        ..Default::default()
    };
    let output = std::path::Path::new("target/screen-proofs");
    fs::create_dir_all(output).unwrap();
    let mut before = original.clone();
    if before
        .background
        .path
        .as_ref()
        .is_some_and(|path| std::path::Path::new(path).is_file())
    {
        before.background.source = BackgroundSource::File;
    }
    let mut proof = sample.clone();
    proof.background = before.background.clone();
    for (name, config) in [("test-before", before), ("telemetry", proof)] {
        let frame = renderer::render(&config, &sensors, &HashMap::new()).unwrap();
        frame.save(output.join(format!("{name}.png"))).unwrap();
        image::imageops::resize(&frame, 960, 640, image::imageops::FilterType::Nearest)
            .save(output.join(format!("{name}-2x.png")))
            .unwrap();
    }
    println!("Created {}", destination.display());
}

#[test]
#[ignore = "manually activates the cloned screen while the app is stopped"]
fn activate_telemetry_screen() {
    let path = persistence::default_config_path();
    let mut current = persistence::load_or_create(&path).unwrap();
    let sample =
        persistence::load_or_create(&persistence::profile_path(&path, NAME).unwrap()).unwrap();
    current.widgets = sample.widgets;
    current.background = sample.background;
    current.theme = sample.theme;
    persistence::save(&path, &current).unwrap();
    persistence::save_active_screen(&path, Some(NAME)).unwrap();
}
