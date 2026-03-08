use crate::io::TcpServer;
use crate::traits::IoInstance;
use chrono::Local;
use mio::{Poll, Token};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorDirection {
    Rx,
    Tx,
}

impl MonitorDirection {
    fn as_str(&self) -> &'static str {
        match self {
            MonitorDirection::Rx => "RX",
            MonitorDirection::Tx => "TX",
        }
    }

    fn swapped_str(&self) -> &'static str {
        match self {
            MonitorDirection::Rx => "TX",
            MonitorDirection::Tx => "RX",
        }
    }
}

pub struct DeviceMonitor {
    server: TcpServer,
    clients: HashMap<Token, Box<dyn IoInstance>>,
    template: String,
    current_direction: Option<MonitorDirection>,
    record_start_time: Option<chrono::DateTime<Local>>,
    token_start: usize,
}

impl DeviceMonitor {
    pub fn new(port: u16, template: String, token_start: usize) -> std::io::Result<Self> {
        Ok(Self {
            server: TcpServer::new(port)?,
            clients: HashMap::new(),
            template,
            current_direction: None,
            record_start_time: None,
            token_start,
        })
    }

    pub fn register(&mut self, poll: &mut Poll, token: Token) -> std::io::Result<()> {
        self.server.register(poll, token)
    }

    pub fn accept(&mut self, poll: &mut Poll) -> std::io::Result<()> {
        while let Some(mut client) = self.server.accept() {
            let token = Token(self.token_start + self.clients.len());
            // In a real implementation we would need a better token management for monitor clients
            // but for now let's use a simplified approach if it's acceptable.
            // Actually, Hub should manage these tokens.
            client.connect(poll, token)?;
            self.clients.insert(token, client);
        }
        Ok(())
    }

    pub fn rx(&mut self, data: &[u8]) {
        self.process_data(MonitorDirection::Rx, data);
    }

    pub fn tx(&mut self, data: &[u8]) {
        self.process_data(MonitorDirection::Tx, data);
    }

    fn process_data(&mut self, dir: MonitorDirection, data: &[u8]) {
        for &b in data {
            let needs_new_record = self.current_direction != Some(dir);
            if needs_new_record {
                self.current_direction = Some(dir);
                self.record_start_time = Some(Local::now());
                // When direction changes, we start fresh.
                // Any pending template prefix will be handled by expand_template called per-char or per-chunk.
            }

            let escaped = escape_char(b);
            let context = TemplateContext {
                direction: dir.as_str(),
                swap_direction: dir.swapped_str(),
                msg: &escaped,
                time: self.record_start_time.unwrap_or_else(Local::now),
            };

            let formatted = expand_template(&self.template, context);
            self.broadcast(&formatted);

            if b == b'\n' {
                // Next char starts a new record
                self.current_direction = None;
            }
        }
    }

    fn broadcast(&mut self, msg: &str) {
        let mut disconnected = Vec::new();
        for (token, client) in self.clients.iter_mut() {
            if client.write_all(msg.as_bytes()) == 0 {
                disconnected.push(*token);
            }
        }
        for token in disconnected {
            self.clients.remove(&token);
        }
    }
}

struct TemplateContext<'a> {
    direction: &'a str,
    swap_direction: &'a str,
    msg: &'a str,
    time: chrono::DateTime<Local>,
}

fn expand_template(template: &str, ctx: TemplateContext) -> String {
    let mut expanded = String::new();
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek() {
                Some('%') => {
                    expanded.push('%');
                    chars.next();
                }
                Some('m') => {
                    expanded.push_str(ctx.msg);
                    chars.next();
                }
                Some('d') => {
                    expanded.push_str(ctx.direction);
                    chars.next();
                }
                Some('D') => {
                    expanded.push_str(ctx.swap_direction);
                    chars.next();
                }
                Some('y') => {
                    expanded.push_str(&ctx.time.format("%Y-%m-%d").to_string());
                    chars.next();
                }
                Some('t') => {
                    expanded.push_str(&ctx.time.format("%H:%M:%S").to_string());
                    chars.next();
                }
                _ => expanded.push('%'),
            }
        } else {
            expanded.push(c);
        }
    }
    expanded
}

fn escape_char(c: u8) -> String {
    match c {
        b'\n' => "\\n".to_string(),
        b'\r' => "\\r".to_string(),
        b'\t' => "\\t".to_string(),
        b'\\' => "\\\\".to_string(),
        0x20..=0x7e => (c as char).to_string(),
        _ => format!("\\x{:02x}", c),
    }
}
