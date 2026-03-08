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
    record_active: bool,
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
            record_active: false,
        })
    }

    pub fn register(&mut self, poll: &mut Poll, token: Token) -> std::io::Result<()> {
        self.server.register(poll, token)
    }

    pub fn accept(&mut self, poll: &mut Poll) -> std::io::Result<()> {
        while let Some(mut client) = self.server.accept() {
            let token = Token(self.token_start + self.clients.len());
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
        let (prefix_template, postfix_template) = split_template(&self.template);

        for &b in data {
            let direction_changed = self.current_direction != Some(dir);

            if !self.record_active || direction_changed {
                if self.record_active && direction_changed {
                    // Close previous record if direction changed before a newline
                    let ctx = self.make_context("", self.current_direction.unwrap());
                    let postfix = expand_template(&postfix_template, ctx);
                    self.broadcast(&postfix);
                }

                self.current_direction = Some(dir);
                self.record_start_time = Some(Local::now());
                self.record_active = true;

                let ctx = self.make_context("", dir);
                let prefix = expand_template(&prefix_template, ctx);
                self.broadcast(&prefix);
            }

            let escaped = escape_char(b);
            self.broadcast(&escaped);

            if b == b'\n' {
                let ctx = self.make_context("", dir);
                let postfix = expand_template(&postfix_template, ctx);
                self.broadcast(&postfix);
                self.record_active = false;
                self.current_direction = None;
            }
        }
    }

    fn make_context<'a>(&self, msg: &'a str, dir: MonitorDirection) -> TemplateContext<'a> {
        TemplateContext {
            direction: dir.as_str(),
            swap_direction: dir.swapped_str(),
            msg,
            time: self.record_start_time.unwrap_or_else(Local::now),
        }
    }

    fn broadcast(&mut self, msg: &str) {
        if msg.is_empty() {
            return;
        }
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

fn split_template(template: &str) -> (String, String) {
    if let Some(idx) = template.find("%m") {
        let prefix = &template[..idx];
        let postfix = &template[idx + 2..];
        (prefix.to_string(), postfix.to_string())
    } else {
        (template.to_string(), String::new())
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_template() {
        assert_eq!(
            split_template("[%t] %d: %m\n"),
            ("[%t] %d: ".to_string(), "\n".to_string())
        );
        assert_eq!(split_template("{%m}"), ("{".to_string(), "}".to_string()));
    }

    #[test]
    fn test_escape_char() {
        assert_eq!(escape_char(b'a'), "a");
        assert_eq!(escape_char(b'\n'), "\\n");
        assert_eq!(escape_char(0x01), "\\x01");
    }
}
