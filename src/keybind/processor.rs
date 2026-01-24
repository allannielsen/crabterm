use std::time::{Duration, Instant};

use super::action::KeybindResult;
use super::config::KeybindConfig;
use super::key::KeyEvent;
use super::parser::{KeyParser, ParseResult};

const ESCAPE_TIMEOUT: Duration = Duration::from_millis(50);
const PREFIX_TIMEOUT: Duration = Duration::from_millis(2000);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    AwaitingPrefixCommand,
}

pub struct KeybindProcessor {
    config: KeybindConfig,
    parser: KeyParser,
    state: State,
    state_entered: Instant,
    last_input: Instant,
}

impl KeybindProcessor {
    pub fn new(config: KeybindConfig) -> Self {
        let now = Instant::now();
        Self {
            config,
            parser: KeyParser::new(),
            state: State::Normal,
            state_entered: now,
            last_input: now,
        }
    }

    /// Process input bytes and return results
    /// May return multiple results if input contains multiple keys
    pub fn process(&mut self, input: &[u8]) -> Vec<KeybindResult> {
        self.last_input = Instant::now();
        self.parser.push(input);
        self.drain_results()
    }

    /// Check for timeouts and return any pending results
    pub fn tick(&mut self) -> Vec<KeybindResult> {
        let now = Instant::now();
        let mut results = Vec::new();

        // Check escape sequence timeout
        if self.parser.has_pending() && now.duration_since(self.last_input) > ESCAPE_TIMEOUT {
            // Force parse pending bytes
            while self.parser.has_pending() {
                if let Some(parse_result) = self.parser.force_parse_first() {
                    if let Some(result) = self.handle_parse_result(parse_result) {
                        results.push(result);
                    }
                } else {
                    break;
                }
            }
        }

        // Check prefix mode timeout
        if self.state == State::AwaitingPrefixCommand
            && now.duration_since(self.state_entered) > PREFIX_TIMEOUT
        {
            // Timeout - forward the original prefix key and reset
            if let Some(prefix) = &self.config.prefix
                && let Some(bytes) = key_event_to_bytes(prefix)
            {
                results.push(KeybindResult::Passthrough(bytes));
            }
            self.state = State::Normal;
        }

        results
    }

    fn drain_results(&mut self) -> Vec<KeybindResult> {
        let mut results = Vec::new();

        loop {
            let parse_result = self.parser.parse_next();
            match parse_result {
                ParseResult::NeedMore => break,
                _ => {
                    if let Some(result) = self.handle_parse_result(parse_result) {
                        results.push(result);
                    }
                }
            }
        }

        results
    }

    fn handle_parse_result(&mut self, parse_result: ParseResult) -> Option<KeybindResult> {
        match parse_result {
            ParseResult::Key(key_event, _) => self.handle_key_event(key_event),
            ParseResult::Passthrough(byte) => Some(KeybindResult::Passthrough(vec![byte])),
            ParseResult::NeedMore => None,
        }
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) -> Option<KeybindResult> {
        match self.state {
            State::Normal => self.handle_normal(key_event),
            State::AwaitingPrefixCommand => self.handle_prefix_mode(key_event),
        }
    }

    fn handle_normal(&mut self, key_event: KeyEvent) -> Option<KeybindResult> {
        // Check direct bindings first
        if let Some(action) = self.config.direct_bindings.get(&key_event) {
            return Some(KeybindResult::Action(action.clone()));
        }

        // Check if this is the prefix key
        if let Some(prefix) = &self.config.prefix
            && key_event == *prefix
        {
            self.state = State::AwaitingPrefixCommand;
            self.state_entered = Instant::now();
            return Some(KeybindResult::Consumed);
        }

        // Pass through
        key_event_to_bytes(&key_event).map(KeybindResult::Passthrough)
    }

    fn handle_prefix_mode(&mut self, key_event: KeyEvent) -> Option<KeybindResult> {
        self.state = State::Normal;

        // Check prefix bindings
        if let Some(action) = self.config.prefix_bindings.get(&key_event) {
            return Some(KeybindResult::Action(action.clone()));
        }

        // Unbound key in prefix mode - forward prefix + this key
        let mut bytes = Vec::new();
        if let Some(prefix) = &self.config.prefix
            && let Some(prefix_bytes) = key_event_to_bytes(prefix)
        {
            bytes.extend(prefix_bytes);
        }
        if let Some(key_bytes) = key_event_to_bytes(&key_event) {
            bytes.extend(key_bytes);
        }

        if bytes.is_empty() {
            None
        } else {
            Some(KeybindResult::Passthrough(bytes))
        }
    }
}

/// Convert a KeyEvent back to terminal bytes (best effort)
fn key_event_to_bytes(event: &KeyEvent) -> Option<Vec<u8>> {
    use super::key::Key;

    let mut bytes = Vec::new();

    // Handle Alt modifier by prepending ESC
    if event.modifiers.alt {
        bytes.push(0x1b);
    }

    match &event.key {
        Key::Char(c) => {
            if event.modifiers.ctrl {
                // Ctrl+A = 0x01, etc.
                let code = c.to_ascii_lowercase() as u8;
                if code.is_ascii_lowercase() {
                    bytes.push(code - b'a' + 1);
                } else {
                    return None;
                }
            } else {
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf);
                bytes.extend_from_slice(encoded.as_bytes());
            }
        }
        Key::Escape => bytes.push(0x1b),
        Key::Enter => bytes.push(0x0d),
        Key::Tab => bytes.push(0x09),
        Key::Backspace => bytes.push(0x7f),
        Key::Up => bytes.extend_from_slice(b"\x1b[A"),
        Key::Down => bytes.extend_from_slice(b"\x1b[B"),
        Key::Right => bytes.extend_from_slice(b"\x1b[C"),
        Key::Left => bytes.extend_from_slice(b"\x1b[D"),
        Key::Home => bytes.extend_from_slice(b"\x1b[H"),
        Key::End => bytes.extend_from_slice(b"\x1b[F"),
        Key::PageUp => bytes.extend_from_slice(b"\x1b[5~"),
        Key::PageDown => bytes.extend_from_slice(b"\x1b[6~"),
        Key::Insert => bytes.extend_from_slice(b"\x1b[2~"),
        Key::Delete => bytes.extend_from_slice(b"\x1b[3~"),
        Key::F(n) => match n {
            1 => bytes.extend_from_slice(b"\x1bOP"),
            2 => bytes.extend_from_slice(b"\x1bOQ"),
            3 => bytes.extend_from_slice(b"\x1bOR"),
            4 => bytes.extend_from_slice(b"\x1bOS"),
            5 => bytes.extend_from_slice(b"\x1b[15~"),
            6 => bytes.extend_from_slice(b"\x1b[17~"),
            7 => bytes.extend_from_slice(b"\x1b[18~"),
            8 => bytes.extend_from_slice(b"\x1b[19~"),
            9 => bytes.extend_from_slice(b"\x1b[20~"),
            10 => bytes.extend_from_slice(b"\x1b[21~"),
            11 => bytes.extend_from_slice(b"\x1b[23~"),
            12 => bytes.extend_from_slice(b"\x1b[24~"),
            _ => return None,
        },
    }

    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybind::Action;

    fn make_config() -> KeybindConfig {
        let mut config = KeybindConfig::new();
        config.prefix = Some(KeyEvent::ctrl_char('a'));
        config
            .prefix_bindings
            .insert(KeyEvent::char('q'), Action::Quit);
        config
            .direct_bindings
            .insert(KeyEvent::ctrl_char('q'), Action::Quit);
        config
    }

    #[test]
    fn test_direct_binding() {
        let mut processor = KeybindProcessor::new(make_config());
        let results = processor.process(&[0x11]); // Ctrl+Q
        assert_eq!(results, vec![KeybindResult::Action(Action::Quit)]);
    }

    #[test]
    fn test_prefix_binding() {
        let mut processor = KeybindProcessor::new(make_config());

        // Press prefix (Ctrl+A)
        let results = processor.process(&[0x01]);
        assert_eq!(results, vec![KeybindResult::Consumed]);

        // Press q
        let results = processor.process(b"q");
        assert_eq!(results, vec![KeybindResult::Action(Action::Quit)]);
    }

    #[test]
    fn test_passthrough() {
        let mut processor = KeybindProcessor::new(make_config());
        let results = processor.process(b"x");
        assert_eq!(results, vec![KeybindResult::Passthrough(b"x".to_vec())]);
    }

    #[test]
    fn test_unbound_prefix_key() {
        let mut processor = KeybindProcessor::new(make_config());

        // Press prefix
        let results = processor.process(&[0x01]);
        assert_eq!(results, vec![KeybindResult::Consumed]);

        // Press unbound key
        let results = processor.process(b"x");
        // Should forward both prefix bytes and the key
        assert_eq!(results, vec![KeybindResult::Passthrough(vec![0x01, b'x'])]);
    }
}
