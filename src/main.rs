use clap::{Arg, Command, value_parser};
use flexi_logger::{DeferredNow, FileSpec, LevelFilter, Logger, Record, WriteMode};
use log::info;
use std::io::Write;

fn log_format(
    w: &mut dyn std::io::Write,
    now: &mut DeferredNow,
    record: &Record,
) -> std::io::Result<()> {
    let module = record.module_path().unwrap_or("?");
    let module_short = module.strip_prefix("crabterm::").unwrap_or(module);
    let t = now.now();
    let micros = t.timestamp_subsec_micros();
    write!(
        w,
        "{}.{:03}.{:03} {} [{}:{}] {}",
        t.format("%y-%m-%d %H:%M:%S"),
        micros / 1000,
        micros % 1000,
        record.level(),
        module_short,
        record.line().unwrap_or(0),
        record.args()
    )
}

/// Log format for console/stderr output - uses \r\n for raw terminal mode compatibility
fn log_format_console(
    w: &mut dyn std::io::Write,
    now: &mut DeferredNow,
    record: &Record,
) -> std::io::Result<()> {
    let module = record.module_path().unwrap_or("?");
    let module_short = module.strip_prefix("crabterm::").unwrap_or(module);
    let t = now.now();
    let micros = t.timestamp_subsec_micros();
    write!(
        w,
        "{}.{:03}.{:03} {} [{}:{}] {}\r",
        t.format("%y-%m-%d %H:%M:%S"),
        micros / 1000,
        micros % 1000,
        record.level(),
        module_short,
        record.line().unwrap_or(0),
        record.args()
    )
}
use std::net::SocketAddr;
use std::panic;
use std::path::PathBuf;

mod hub;
mod io;
mod iofilter;
mod keybind;
mod term;
mod traits;

use hub::IoHub;
use io::Console;
use io::EchoDevice;
use io::SerialDevice;
use io::TcpDevice;
use io::TcpServer;
use iofilter::FilterChain;
use keybind::KeybindConfig;
use term::disable_raw_mode;

use crate::traits::IoInstance;

const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_SHA"), ")");

macro_rules! raw_println {
    ($($arg:tt)*) => {
        print!("{}\r\n", format!($($arg)*));
    };
}

#[derive(Debug, Clone)]
enum DeviceMode {
    Echo(),
    Serial(String),
    Tcp(String),
}

fn parse_device(val: &str) -> Result<DeviceMode, String> {
    if val.starts_with("/dev/") {
        return Ok(DeviceMode::Serial(val.to_string()));
    }

    if val.starts_with("echo") {
        return Ok(DeviceMode::Echo());
    }

    if let Some((host, port_str)) = val.split_once(':')
        && !host.is_empty()
        && !port_str.is_empty()
    {
        return Ok(DeviceMode::Tcp(val.to_string()));
    }

    Err(String::from(
        "Invalid device format. Use /dev/ttyUSB0, hostname:port, echo",
    ))
}

fn main() -> std::io::Result<()> {
    panic::set_hook(Box::new(|info| {
        // Attempt to restore terminal
        let _ = disable_raw_mode();

        // Print panic message with \r\n
        let _ = writeln!(std::io::stderr(), "\nPanic occurred: {}\n", info);
    }));

    // Collect args before parsing for logging
    let args: Vec<String> = std::env::args().collect();

    let dev_help = "Device - /dev/rs232-device|(ip-address|hostname):port|echo";
    let matches = Command::new("crabterm")
        .version(VERSION)
        .author("Allan W. Nielsen")
        .about("A terminal (uart) server and client")
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .value_name("CONFIG_PATH")
                .help("Path to config file (default: ~/.crabterm)")
                .value_parser(clap::value_parser!(PathBuf))
                .num_args(1),
        )
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .value_name("PORT")
                .help("Open a TCP server and listen on port")
                .value_parser(value_parser!(u16)),
        )
        .arg(
            Arg::new("baudrate")
                .short('b')
                .long("baudrate")
                .value_name("BAUDRATE")
                .help("Baudrate")
                .default_value("115200")
                .value_parser(value_parser!(u32)),
        )
        .arg(
            Arg::new("headless")
                .long("headless")
                .help("Headless/daemon mode - IO not printed locally (only useful along with -p)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("devicepos")
                .index(1)
                .value_name("DEVICE")
                .conflicts_with("device")
                .help(dev_help)
                .value_parser(parse_device)
                .num_args(1),
        )
        .arg(
            Arg::new("device")
                .short('d')
                .long("device")
                .value_name("DEVICE")
                .help(dev_help)
                .value_parser(parse_device)
                .num_args(1),
        )
        .arg(
            Arg::new("log-file")
                .short('l')
                .long("log-file")
                .value_name("LOG_PATH")
                .help("Enable logging and write logs to the specified file")
                .value_parser(clap::value_parser!(PathBuf))
                .num_args(1),
        )
        .arg(
            Arg::new("log-level")
                .short('L')
                .long("log-level")
                .value_name("LOG_LEVEL")
                .help("Set the log level (error, warn, info, debug, trace)")
                .value_parser(clap::value_parser!(LevelFilter))
                .default_value("info")
                .num_args(1),
        )
        .arg(
            Arg::new("no-announce")
                .long("no-announce")
                .help("Suppress informational messages (connect/disconnect) to clients")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .help("Enable console logging (-v=error, -vv=warn, -vvv=info, -vvvv=debug, -vvvvv=trace)")
                .action(clap::ArgAction::Count),
        )
        .get_matches();

    // Handle verbose flag - map count to log level
    let verbose_count = matches.get_count("verbose");
    let verbose_level = match verbose_count {
        0 => None,
        1 => Some(LevelFilter::Error),
        2 => Some(LevelFilter::Warn),
        3 => Some(LevelFilter::Info),
        4 => Some(LevelFilter::Debug),
        _ => Some(LevelFilter::Trace),
    };

    // Configure logging
    if let Some(path) = matches.get_one::<PathBuf>("log-file") {
        let file_level = matches.get_one::<LevelFilter>("log-level").unwrap();

        // If verbose is enabled, use the more verbose of the two levels
        let effective_level = if let Some(vlevel) = verbose_level {
            std::cmp::max(*file_level, vlevel)
        } else {
            *file_level
        };

        let mut logger = Logger::try_with_str(effective_level.as_str())
            .unwrap()
            .log_to_file(FileSpec::try_from(path).expect("Invalid log path"))
            .format_for_files(log_format)
            .append()
            .write_mode(WriteMode::Direct);

        // If verbose is enabled, also duplicate to stderr with console format
        if verbose_level.is_some() {
            logger = logger
                .duplicate_to_stderr(flexi_logger::Duplicate::All)
                .format_for_stderr(log_format_console);
        }

        logger.start().unwrap();
    } else if let Some(vlevel) = verbose_level {
        // No log file, but verbose is enabled - log to stderr with console format
        Logger::try_with_str(vlevel.as_str())
            .unwrap()
            .format(log_format_console)
            .write_mode(WriteMode::Direct)
            .start()
            .unwrap();
    }

    info!("Starting crabterm");
    info!("Command line: {}", args.join(" "));

    let mut server: Option<TcpServer> = None;
    if let Some(port) = matches.get_one::<u16>("port") {
        raw_println!("Listning at port: {}", port);
        server = Some(TcpServer::new(*port)?);
    }

    let device: Box<dyn IoInstance> = if let Some(dev) = matches
        .get_one::<DeviceMode>("device")
        .or_else(|| matches.get_one::<DeviceMode>("devicepos"))
    {
        match dev {
            DeviceMode::Serial(path) => {
                let baudrate = matches.get_one::<u32>("baudrate").unwrap();
                // raw_println!("Serial device: {}, baudrate: {}", path, baudrate);
                let client = SerialDevice::new(path.clone(), *baudrate)?;
                Box::new(client)
            }
            DeviceMode::Tcp(addr) => {
                raw_println!("TCP device: {}", addr);

                let addr: SocketAddr = addr.parse().unwrap();
                let client = TcpDevice::new(addr)?;
                Box::new(client)
            }
            DeviceMode::Echo() => {
                raw_println!("Echo mode");
                Box::new(EchoDevice::new()?)
            }
        }
    } else {
        panic!("No device specified");
    };

    let headless = matches.get_flag("headless");

    if headless && server.is_none() {
        raw_println!("Error: --headless requires -p/--port option");
        std::process::exit(1);
    }

    let announce = !matches.get_flag("no-announce");
    let mut hub = IoHub::new(device, server, announce)?;

    if !headless {
        let config = KeybindConfig::load(matches.get_one::<PathBuf>("config").cloned());
        let filter_chain = FilterChain::new(&config.settings);
        let console = Console::new(config, filter_chain)?;
        hub.add(Box::new(console))?;
    }

    loop {
        info!("Main loop: checking quit status");
        if hub.is_quit_requested() {
            info!("Main loop: quit requested, breaking");
            break;
        }
        info!("Main loop: calling hub.run()");
        hub.run()?;
        info!("Main loop: hub.run() returned");
    }

    info!("Main loop exited, shutting down");
    Ok(())
}
