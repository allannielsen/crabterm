use log::info;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use super::action::Action;
use super::key::{Key, KeyEvent, Modifiers};

#[derive(Debug, Clone)]
pub struct KeybindConfig {
    pub prefix: Option<KeyEvent>,
    pub prefix_bindings: HashMap<KeyEvent, Action>,
    pub direct_bindings: HashMap<KeyEvent, Action>,
    pub settings: HashMap<String, bool>,
}

impl Default for KeybindConfig {
    fn default() -> Self {
        let mut config = KeybindConfig {
            prefix: Some(KeyEvent::ctrl_char('a')),
            prefix_bindings: HashMap::new(),
            direct_bindings: HashMap::new(),
            settings: HashMap::new(),
        };

        // Default bindings
        config
            .direct_bindings
            .insert(KeyEvent::ctrl_char('q'), Action::Quit);
        config
            .prefix_bindings
            .insert(KeyEvent::char('q'), Action::Send(vec![0x11])); // Send Ctrl+Q
        config
            .prefix_bindings
            .insert(KeyEvent::ctrl_char('a'), Action::Send(vec![0x01])); // Send literal Ctrl+A
        config
            .prefix_bindings
            .insert(KeyEvent::char('t'), Action::FilterToggle("timestamp".to_string()));

        config
    }
}

impl KeybindConfig {
    pub fn new() -> Self {
        Self {
            prefix: None,
            prefix_bindings: HashMap::new(),
            direct_bindings: HashMap::new(),
            settings: HashMap::new(),
        }
    }

    pub fn load(path: Option<PathBuf>) -> Self {
        let mut config_path = dirs::home_dir().map(|home| home.join(".crabterm"));

        if path.is_some() {
            config_path = path;
        }

        if let Some(p) = config_path
            && p.exists()
        {
            match KeybindConfig::load_from_file(&p) {
                Ok(config) => {
                    info!("Loaded keybind config from {:?}", p);
                    config
                }
                Err(e) => {
                    println!("Warning: Failed to parse {}: {}", p.display(), e);
                    KeybindConfig::default()
                }
            }
        } else {
            KeybindConfig::default()
        }
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::parse(&content)
    }

    pub fn parse(content: &str) -> Result<Self, String> {
        let mut config = KeybindConfig::new();

        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            config
                .parse_line(line)
                .map_err(|e| format!("Line {}: {}", line_num + 1, e))?;
        }

        Ok(config)
    }

    fn parse_line(&mut self, line: &str) -> Result<(), String> {
        let mut parts = LineParser::new(line);

        let directive = parts.next_word().ok_or("Empty directive")?;

        match directive {
            "prefix" => {
                let key_str = parts.next_word().ok_or("Missing key for prefix")?;
                self.prefix = Some(parse_key_event(key_str)?);
            }
            "map-prefix" => {
                let key_str = parts.next_word().ok_or("Missing key for map-prefix")?;
                let key = parse_key_event(key_str)?;
                let action = parse_action(&mut parts)?;
                self.prefix_bindings.insert(key, action);
            }
            "map" => {
                let key_str = parts.next_word().ok_or("Missing key for map")?;
                let key = parse_key_event(key_str)?;
                let action = parse_action(&mut parts)?;
                self.direct_bindings.insert(key, action);
            }
            "set" => {
                let name = parts.next_word().ok_or("Missing setting name")?;
                let value_str = parts.next_word().ok_or("Missing setting value (on/off)")?;
                let value = match value_str.to_lowercase().as_str() {
                    "on" | "true" | "yes" | "1" => true,
                    "off" | "false" | "no" | "0" => false,
                    _ => return Err(format!("Invalid boolean value: {}", value_str)),
                };
                self.settings.insert(name.to_string(), value);
            }
            _ => return Err(format!("Unknown directive: {}", directive)),
        }

        Ok(())
    }
}

struct LineParser<'a> {
    remaining: &'a str,
}

impl<'a> LineParser<'a> {
    fn new(line: &'a str) -> Self {
        Self { remaining: line }
    }

    fn next_word(&mut self) -> Option<&'a str> {
        self.remaining = self.remaining.trim_start();
        if self.remaining.is_empty() {
            return None;
        }

        let end = self
            .remaining
            .find(char::is_whitespace)
            .unwrap_or(self.remaining.len());
        let word = &self.remaining[..end];
        self.remaining = &self.remaining[end..];
        Some(word)
    }

    fn next_quoted_string(&mut self) -> Option<String> {
        self.remaining = self.remaining.trim_start();
        if !self.remaining.starts_with('"') {
            return None;
        }

        self.remaining = &self.remaining[1..]; // Skip opening quote

        let mut result = String::new();
        let mut chars = self.remaining.chars().peekable();
        let mut consumed = 0;

        while let Some(c) = chars.next() {
            consumed += c.len_utf8();
            if c == '"' {
                self.remaining = &self.remaining[consumed..];
                return Some(result);
            } else if c == '\\' {
                if let Some(&next) = chars.peek() {
                    consumed += next.len_utf8();
                    chars.next();
                    match next {
                        'n' => result.push('\n'),
                        'r' => result.push('\r'),
                        't' => result.push('\t'),
                        '\\' => result.push('\\'),
                        '"' => result.push('"'),
                        'x' => {
                            // Parse \xHH
                            let mut hex = String::new();
                            for _ in 0..2 {
                                if let Some(&h) = chars.peek()
                                    && h.is_ascii_hexdigit()
                                {
                                    hex.push(h);
                                    consumed += h.len_utf8();
                                    chars.next();
                                }
                            }
                            if hex.len() == 2
                                && let Ok(byte) = u8::from_str_radix(&hex, 16)
                            {
                                result.push(byte as char);
                            }
                        }
                        _ => {
                            result.push('\\');
                            result.push(next);
                        }
                    }
                }
            } else {
                result.push(c);
            }
        }

        None // Unterminated string
    }

    fn rest(&self) -> &'a str {
        self.remaining.trim()
    }
}

fn parse_key_event(s: &str) -> Result<KeyEvent, String> {
    let mut modifiers = Modifiers::none();
    let parts: Vec<&str> = s.split('+').collect();

    if parts.is_empty() {
        return Err("Empty key specification".to_string());
    }

    // Parse modifiers (all but the last part)
    for part in &parts[..parts.len() - 1] {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" | "c" => modifiers.ctrl = true,
            "alt" | "meta" | "m" => modifiers.alt = true,
            "shift" | "s" => modifiers.shift = true,
            _ => return Err(format!("Unknown modifier: {}", part)),
        }
    }

    // Parse the key (last part)
    let key_str = parts.last().unwrap();
    let key = parse_key(key_str)?;

    Ok(KeyEvent::new(key, modifiers))
}

fn parse_key(s: &str) -> Result<Key, String> {
    let lower = s.to_lowercase();

    // Function keys
    if lower.starts_with('f')
        && lower.len() > 1
        && let Ok(n) = lower[1..].parse::<u8>()
        && (1..=12).contains(&n)
    {
        return Ok(Key::F(n));
    }

    // Named keys
    match lower.as_str() {
        "escape" | "esc" => return Ok(Key::Escape),
        "enter" | "return" | "cr" => return Ok(Key::Enter),
        "tab" => return Ok(Key::Tab),
        "backspace" | "bs" => return Ok(Key::Backspace),
        "up" => return Ok(Key::Up),
        "down" => return Ok(Key::Down),
        "left" => return Ok(Key::Left),
        "right" => return Ok(Key::Right),
        "home" => return Ok(Key::Home),
        "end" => return Ok(Key::End),
        "pageup" | "pgup" => return Ok(Key::PageUp),
        "pagedown" | "pgdn" => return Ok(Key::PageDown),
        "insert" | "ins" => return Ok(Key::Insert),
        "delete" | "del" => return Ok(Key::Delete),
        "space" => return Ok(Key::Char(' ')),
        _ => {}
    }

    // Single character
    let chars: Vec<char> = s.chars().collect();
    if chars.len() == 1 {
        return Ok(Key::Char(chars[0].to_ascii_lowercase()));
    }

    Err(format!("Unknown key: {}", s))
}

fn parse_action(parts: &mut LineParser) -> Result<Action, String> {
    let action_name = parts.next_word().ok_or("Missing action")?;

    match action_name {
        "quit" => Ok(Action::Quit),
        "filter-toggle" => {
            let filter_name = parts.next_word().ok_or("filter-toggle requires a filter name")?;
            Ok(Action::FilterToggle(filter_name.to_string()))
        }
        "send" => {
            let string = parts
                .next_quoted_string()
                .ok_or("send requires a quoted string")?;
            Ok(Action::Send(string.into_bytes()))
        }
        "send-bytes" => {
            let mut bytes = Vec::new();
            let rest = parts.rest();
            for part in rest.split_whitespace() {
                let byte = if part.starts_with("0x") || part.starts_with("0X") {
                    u8::from_str_radix(&part[2..], 16)
                        .map_err(|_| format!("Invalid hex byte: {}", part))?
                } else {
                    part.parse::<u8>()
                        .map_err(|_| format!("Invalid byte: {}", part))?
                };
                bytes.push(byte);
            }
            if bytes.is_empty() {
                return Err("send-bytes requires at least one byte".to_string());
            }
            Ok(Action::Send(bytes))
        }
        _ => Err(format!("Unknown action: {}", action_name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_config() {
        let config = KeybindConfig::parse(
            r#"
            # This is a comment
            prefix Ctrl+a
            map-prefix q quit
            map Ctrl+q quit
        "#,
        )
        .unwrap();

        assert_eq!(config.prefix, Some(KeyEvent::ctrl_char('a')));
        assert_eq!(
            config.prefix_bindings.get(&KeyEvent::char('q')),
            Some(&Action::Quit)
        );
        assert_eq!(
            config.direct_bindings.get(&KeyEvent::ctrl_char('q')),
            Some(&Action::Quit)
        );
    }

    #[test]
    fn test_parse_send_action() {
        let config = KeybindConfig::parse(
            r#"
            map-prefix s send "hello\r\n"
        "#,
        )
        .unwrap();

        assert_eq!(
            config.prefix_bindings.get(&KeyEvent::char('s')),
            Some(&Action::Send(b"hello\r\n".to_vec()))
        );
    }

    #[test]
    fn test_parse_send_bytes() {
        let config = KeybindConfig::parse(
            r#"
            map-prefix e send-bytes 0x1b 0x4f
        "#,
        )
        .unwrap();

        assert_eq!(
            config.prefix_bindings.get(&KeyEvent::char('e')),
            Some(&Action::Send(vec![0x1b, 0x4f]))
        );
    }

    #[test]
    fn test_parse_key_with_modifiers() {
        let key = parse_key_event("Ctrl+Shift+a").unwrap();
        assert!(key.modifiers.ctrl);
        assert!(key.modifiers.shift);
        assert!(!key.modifiers.alt);
        assert_eq!(key.key, Key::Char('a'));
    }

    #[test]
    fn test_parse_function_key() {
        let key = parse_key_event("Alt+F1").unwrap();
        assert!(key.modifiers.alt);
        assert_eq!(key.key, Key::F(1));
    }
}
