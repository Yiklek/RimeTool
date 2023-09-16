#![allow(unused)]
#![windows_subsystem = "windows"]

use log::{debug, error, info, trace, warn, LevelFilter};
use log4rs::{
    self,
    append::{
        console::{ConsoleAppender, Target},
        file::FileAppender,
        rolling_file::{
            policy::{
                self,
                compound::{
                    roll::fixed_window::FixedWindowRoller, trigger::size::SizeTrigger,
                    CompoundPolicy,
                },
            },
            RollingFileAppender,
        },
    },
    config::{Appender, Root},
    encode::pattern::PatternEncoder,
    filter::threshold::ThresholdFilter,
    Config,
};
use serde::Deserialize;
use serde_yaml;
use std::fs;
use std::{
    borrow::BorrowMut,
    cell::RefCell,
    sync::{Arc, Mutex},
    thread,
};
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};
use sysinfo::{ProcessExt, System, SystemExt};
use tao::event_loop::{ControlFlow, DeviceEventFilter, EventLoopBuilder};
use toml;
use tray_icon::{
    menu::{
        accelerator::Accelerator, AboutMetadata, CheckMenuItem, CheckMenuItemBuilder,
        Icon as MIcon, Menu, MenuEvent, MenuItem, PredefinedMenuItem,
    },
    Icon, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

#[derive(Debug)]
enum TrayUserEvent {
    Quit,
    ServiceClicked,
    IconClicked,
    UpdateService(bool),
}

#[derive(Debug, Clone)]
struct RgbaIcon {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}
#[derive(Clone, Debug, Deserialize)]
struct ToolConfig {
    #[serde(default = "default_rime_root")]
    root: String,
}

const NAME: &str = "Rime 工具箱";
const ICON_BYTES: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icon.png"));
static CONFIG: Lazy<ToolConfig> = Lazy::new(|| load_config());

#[cfg(target_os = "windows")]
fn default_rime_root() -> String {
    use registry::{key::Error, Hive, RegKey, Security};
    Hive::LocalMachine
        .open(r"SOFTWARE\WOW6432Node\Rime\Weasel", Security::Read)
        .ok()
        .and_then(|key| key.value("WeaselRoot").ok())
        .map(|v| v.to_string())
        .unwrap_or(String::from("C:/Program Files (x86)/Rime/weasel-0.15.0"))
}

#[cfg(target_os = "linux")]
fn default_rime_root() -> String {
    "/usr/local".to_string()
}

use once_cell::sync::Lazy;

fn init_log_from_file() -> bool {
    let log_config_paths = vec!["config/log4rs.toml", "config/log4rs.yaml"];
    log_config_paths
        .iter()
        .map(|p| Path::new(p))
        .find(|p| p.exists())
        .and_then(|p| log4rs::init_file(p, Default::default()).ok())
        .is_some()
}
fn init_log() {
    if !init_log_from_file() {
        let stderr = ConsoleAppender::builder()
            .encoder(Box::new(PatternEncoder::new(
                "{d(%Y-%m-%d %H:%M:%S %Z)} {M}:{f}:{L} {l} {T} {t} - {m}{n}",
            )))
            .target(Target::Stderr)
            .build();
        let log_path = env::temp_dir().join(format!("{}.log", env!("CARGO_PKG_NAME")));

        let window_size = 5;
        let fixed_window_roller = FixedWindowRoller::builder()
            .build(
                &format!("{}.{{}}.log", log_path.to_string_lossy()),
                window_size,
            )
            .unwrap();
        let size_limit = 5 * 1024 * 1024; // 5MB as max log file size to roll
        let size_trigger = SizeTrigger::new(size_limit);
        let compound_policy =
            CompoundPolicy::new(Box::new(size_trigger), Box::new(fixed_window_roller));
        let logfile = RollingFileAppender::builder()
            .encoder(Box::new(PatternEncoder::new(
                "{d(%Y-%m-%d %H:%M:%S %Z)} {M}:{f}:{L} {l} {T} {t} - {m}{n}",
            )))
            .build(log_path, Box::new(compound_policy))
            .unwrap();
        let config = Config::builder()
            .appender(Appender::builder().build("logfile", Box::new(logfile)))
            .appender(
                Appender::builder()
                    .filter(Box::new(ThresholdFilter::new(LevelFilter::Debug)))
                    .build("stderr", Box::new(stderr)),
            )
            .build(
                Root::builder()
                    .appender("logfile")
                    .appender("stderr")
                    .build(LevelFilter::Trace),
            )
            .unwrap();
        let _handle = log4rs::init_config(config).unwrap();
    }
}
fn load_config() -> ToolConfig {
    let mut config_path = Some(PathBuf::from("config/config.toml"));
    let config_str = config_path
        .and_then(|p| {
            if p.exists() {
                Some(p)
            } else {
                env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|p| p.to_owned()))
                    .map(|p| p.join("config").join("config.toml"))
            }
        })
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default();
    toml::from_str(&config_str).expect("parse config toml failed.")
}

fn rime_redeploy() {
    #[cfg(target_os = "windows")]
    let args = vec!["/deploy"];

    #[cfg(target_os = "windows")]
    let deploy = "WeaselDeployer.exe";

    let deployer = Path::new(&CONFIG.root).join(deploy);

    thread::spawn(move || {
        let redeploy = Command::new(deployer.clone()).args(args).spawn();
        if let Err(e) = redeploy {
            error!(
                "failed to deploy. {:?} {}",
                deployer.to_str().unwrap_or_default(),
                e
            );
        }
    });
}
fn rime_start_service() {
    #[cfg(target_os = "windows")]
    let args = vec!["/restart"];

    #[cfg(target_os = "windows")]
    let server = "WeaselServer.exe";

    let server = Path::new(&CONFIG.root).join(server);

    thread::spawn(move || {
        let redeploy = Command::new(server.clone()).args(args).spawn();
        if let Err(e) = redeploy {
            error!(
                "failed to restart. {} {}",
                server.to_str().unwrap_or_default(),
                e
            );
        }
    });
}
fn rime_stop_service() {
    let s = System::new_all();
    let ps = s.processes_by_name("WeaselServer.exe");
    for p in ps {
        p.kill();
    }
}
fn get_service_status() -> bool {
    let s = System::new_all();
    let ps = s.processes_by_name("WeaselServer.exe");
    ps.count() > 0
}
fn update_service_status(service_item: &CheckMenuItem) {
    service_item.set_checked(get_service_status());
}
fn toggle_service(checked: bool) {
    if checked {
        rime_start_service();
    } else {
        rime_stop_service();
    }
}

fn main() {
    init_log();
    let s = System::new_all();

    let e = env::current_exe()
        .ok()
        .and_then(|p| {
            p.file_name()
                .and_then(|f| f.to_str())
                .map(|f| String::from(f))
        })
        .unwrap_or(format!(env!("CARGO_PKG_NAME")));

    let ps = s.processes_by_name(&e);
    if ps.count() > 1 {
        warn!("{} is already running. exit.", e);
        std::process::exit(0);
    }
    info!("start...");

    let icon = ICON_BYTES;
    let event_loop = EventLoopBuilder::<TrayUserEvent>::with_user_event().build();

    let tray_menu = Menu::new();

    let icon_about = load_icon(icon);
    let icon_exe = icon_about.clone();
    let icon_about =
        MIcon::from_rgba(icon_about.rgba, icon_about.width, icon_about.height).expect("Fail icon");
    let icon_exe =
        Icon::from_rgba(icon_exe.rgba, icon_exe.width, icon_exe.height).expect("Fail icon");

    let service = CheckMenuItem::new("算法服务", true, true, None);
    let redeploy = MenuItem::new("重新部署", true, None);
    let quit = MenuItem::new("退出", true, None);
    tray_menu.append_items(&[
        &service,
        &redeploy,
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::about(
            Some("关于"),
            Some(AboutMetadata {
                name: Some(format!("{}", NAME)),
                copyright: Some(format!(
                    "Copyright Yiklek. {}",
                    env!("CARGO_PKG_REPOSITORY")
                )),
                icon: Some(icon_about),
                version: Some(format!(env!("CARGO_PKG_VERSION"))),
                ..Default::default()
            }),
        ),
        &quit,
    ]);

    let mut tray_icon = Some(
        TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_tooltip(NAME)
            .with_icon(icon_exe)
            .with_menu_on_left_click(true)
            .build()
            .expect("Build TrayIcon Failed."),
    );
    let quit_id = quit.id().clone();
    let redeploy_id = redeploy.id().clone();
    let service_id = service.id().clone();
    debug!(
        "ids: quit: {:?} redeploy: {:?} service: {:?}",
        quit_id, redeploy_id, service_id
    );
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        trace!("menu {event:?}");
        if event.id == quit_id {
            proxy.send_event(TrayUserEvent::Quit);
        } else if event.id == redeploy_id {
            rime_redeploy();
        } else if event.id == service_id {
            proxy.send_event(TrayUserEvent::ServiceClicked);
        }
    }));

    let proxy = event_loop.create_proxy();
    let service_ptr = &service as *const CheckMenuItem as usize;
    TrayIconEvent::set_event_handler(Some(move |e| {
        trace!("tray {e:?}");

        unsafe {
            update_service_status(&*(service_ptr as *const CheckMenuItem));
        }
        proxy.send_event(TrayUserEvent::IconClicked);
    }));
    // filter all device event, maybe change to unfocused, if add another feature.
    event_loop.set_device_event_filter(DeviceEventFilter::Always);
    event_loop.run(move |event, _, control_flow| {
        use tao::event::Event::NewEvents;
        use tao::event::Event::UserEvent;

        *control_flow = ControlFlow::Wait;
        // trace!("loop {event:?}");
        match event {
            UserEvent(TrayUserEvent::Quit) => {
                debug!("quit.");
                tray_icon.take();
                *control_flow = ControlFlow::Exit;
            }
            UserEvent(TrayUserEvent::IconClicked) => {
                update_service_status(&service);
            }
            UserEvent(TrayUserEvent::ServiceClicked) => {
                toggle_service(service.is_checked());
                update_service_status(&service);
            }
            _ => {}
        }
    })
}

fn load_icon(icon: &[u8]) -> RgbaIcon {
    let image = image::load_from_memory(icon)
        .expect("Failed to open icon path")
        .into_rgba8();
    let (width, height) = image.dimensions();
    let rgba = image.into_raw();
    RgbaIcon {
        rgba,
        width,
        height,
    }
}
