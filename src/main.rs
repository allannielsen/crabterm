use clap::{Arg, Command, value_parser};
use flexi_logger::{FileSpec, LevelFilter, Logger, WriteMode};
use log::info;
use std::io::Write;
use std::net::SocketAddr;
use std::panic;
use std::path::PathBuf;

mod hub;
mod io;
mod keybind;
mod term;
mod traits;

use hub::IoHub;
use io::Console;
use io::EchoDevice;
use io::SerialDevice;
use io::TcpDevice;
use io::TcpServer;
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
        .get_matches();

    if let Some(path) = matches.get_one::<PathBuf>("log-file") {
        let level = matches.get_one::<LevelFilter>("log-level").unwrap();
        Logger::try_with_str(level.as_str())
            .unwrap()
            .log_to_file(FileSpec::try_from(path).expect("Invalid log path"))
            .append()
            .write_mode(WriteMode::BufferAndFlush)
            .start()
            .unwrap();
    }

    info!("Starting crabterm");

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

    let mut hub = IoHub::new(device, server)?;

    if !headless {
        let console = Console::new(KeybindConfig::load(
            matches.get_one::<PathBuf>("config").cloned(),
        ))?;
        hub.add(Box::new(console))?;
    }

    loop {
        if hub.is_quit_requested() {
            break;
        }
        hub.run()?
    }

    Ok(())
}
