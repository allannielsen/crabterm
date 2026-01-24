use super::key::{Key, KeyEvent, Modifiers};

/// Result of parsing bytes
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseResult {
    /// Successfully parsed a key event, consumed `bytes_consumed` bytes
    Key(KeyEvent, usize),
    /// Need more bytes to determine the key (e.g., after receiving ESC)
    NeedMore,
    /// No valid key sequence found, pass through first byte
    Passthrough(u8),
}

/// Parse raw terminal input bytes into key events
pub struct KeyParser {
    buffer: Vec<u8>,
}

impl KeyParser {
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Add bytes to the parse buffer
    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// Check if buffer has pending data
    pub fn has_pending(&self) -> bool {
        !self.buffer.is_empty()
    }

    /// Get pending bytes (for passthrough on timeout)
    #[allow(dead_code)]
    pub fn take_pending(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buffer)
    }

    /// Try to parse the next key event from the buffer
    pub fn parse_next(&mut self) -> ParseResult {
        if self.buffer.is_empty() {
            return ParseResult::NeedMore;
        }

        let result = parse_bytes(&self.buffer);

        match result {
            ParseResult::Key(key, consumed) => {
                self.buffer.drain(..consumed);
                ParseResult::Key(key, consumed)
            }
            ParseResult::Passthrough(b) => {
                self.buffer.remove(0);
                ParseResult::Passthrough(b)
            }
            ParseResult::NeedMore => ParseResult::NeedMore,
        }
    }

    /// Force interpret the first byte as a standalone key (used after timeout)
    pub fn force_parse_first(&mut self) -> Option<ParseResult> {
        if self.buffer.is_empty() {
            return None;
        }

        let byte = self.buffer[0];

        // If it's ESC alone, return Escape key
        if byte == 0x1b {
            self.buffer.remove(0);
            return Some(ParseResult::Key(
                KeyEvent::new(Key::Escape, Modifiers::none()),
                1,
            ));
        }

        // Otherwise parse normally
        Some(self.parse_next())
    }
}

impl Default for KeyParser {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_bytes(bytes: &[u8]) -> ParseResult {
    if bytes.is_empty() {
        return ParseResult::NeedMore;
    }

    let first = bytes[0];

    // Escape sequences
    if first == 0x1b {
        return parse_escape_sequence(bytes);
    }

    // Control characters (0x01-0x1A except some special ones)
    if (0x01..=0x1a).contains(&first) {
        return match first {
            0x09 => ParseResult::Key(KeyEvent::new(Key::Tab, Modifiers::none()), 1),
            0x0d => ParseResult::Key(KeyEvent::new(Key::Enter, Modifiers::none()), 1),
            _ => {
                // Ctrl+A = 0x01, Ctrl+B = 0x02, ..., Ctrl+Z = 0x1A
                let c = (first + b'a' - 1) as char;
                ParseResult::Key(KeyEvent::new(Key::Char(c), Modifiers::ctrl()), 1)
            }
        };
    }

    // DEL / Backspace
    if first == 0x7f {
        return ParseResult::Key(KeyEvent::new(Key::Backspace, Modifiers::none()), 1);
    }

    // Regular printable ASCII
    if (0x20..0x7f).contains(&first) {
        return ParseResult::Key(
            KeyEvent::new(Key::Char(first as char), Modifiers::none()),
            1,
        );
    }

    // High bytes - just pass through
    ParseResult::Passthrough(first)
}

fn parse_escape_sequence(bytes: &[u8]) -> ParseResult {
    if bytes.len() < 2 {
        return ParseResult::NeedMore;
    }

    let second = bytes[1];

    // CSI sequences: ESC [
    if second == b'[' {
        return parse_csi_sequence(bytes);
    }

    // SS3 sequences: ESC O (for F1-F4 and some others)
    if second == b'O' {
        return parse_ss3_sequence(bytes);
    }

    // Alt+key: ESC followed by printable character
    if (0x20..0x7f).contains(&second) {
        let key = Key::Char(second as char);
        return ParseResult::Key(KeyEvent::new(key, Modifiers::alt()), 2);
    }

    // Alt+Ctrl+key: ESC followed by control character
    if (0x01..=0x1a).contains(&second) && second != 0x1b {
        let c = (second + b'a' - 1) as char;
        let mut mods = Modifiers::ctrl();
        mods.alt = true;
        return ParseResult::Key(KeyEvent::new(Key::Char(c), mods), 2);
    }

    // Unknown escape sequence - just return ESC as standalone
    ParseResult::Key(KeyEvent::new(Key::Escape, Modifiers::none()), 1)
}

fn parse_csi_sequence(bytes: &[u8]) -> ParseResult {
    // Minimum CSI sequence: ESC [ <final>
    if bytes.len() < 3 {
        return ParseResult::NeedMore;
    }

    // Find the final byte (0x40-0x7E)
    let mut i = 2;
    while i < bytes.len() {
        let b = bytes[i];
        if (0x40..=0x7e).contains(&b) {
            // Found final byte
            let params = &bytes[2..i];
            let final_byte = b;
            return interpret_csi(params, final_byte, i + 1);
        }
        // Intermediate bytes are 0x20-0x2F, parameter bytes are 0x30-0x3F
        if !(0x20..=0x3f).contains(&b) {
            // Invalid sequence
            return ParseResult::Key(KeyEvent::new(Key::Escape, Modifiers::none()), 1);
        }
        i += 1;
    }

    // Need more data
    ParseResult::NeedMore
}

fn interpret_csi(params: &[u8], final_byte: u8, consumed: usize) -> ParseResult {
    let params_str = std::str::from_utf8(params).unwrap_or("");
    let parts: Vec<&str> = params_str.split(';').collect();

    // Parse modifier from second parameter (CSI 1;2A = Shift+Up)
    let modifier = if parts.len() >= 2 {
        parse_modifier_param(parts[1])
    } else {
        Modifiers::none()
    };

    let key = match final_byte {
        b'A' => Some(Key::Up),
        b'B' => Some(Key::Down),
        b'C' => Some(Key::Right),
        b'D' => Some(Key::Left),
        b'H' => Some(Key::Home),
        b'F' => Some(Key::End),
        b'~' => {
            // Tilde sequences: ESC [ <number> ~
            match parts.first().and_then(|s| s.parse::<u8>().ok()) {
                Some(1) => Some(Key::Home),
                Some(2) => Some(Key::Insert),
                Some(3) => Some(Key::Delete),
                Some(4) => Some(Key::End),
                Some(5) => Some(Key::PageUp),
                Some(6) => Some(Key::PageDown),
                Some(15) => Some(Key::F(5)),
                Some(17) => Some(Key::F(6)),
                Some(18) => Some(Key::F(7)),
                Some(19) => Some(Key::F(8)),
                Some(20) => Some(Key::F(9)),
                Some(21) => Some(Key::F(10)),
                Some(23) => Some(Key::F(11)),
                Some(24) => Some(Key::F(12)),
                _ => None,
            }
        }
        _ => None,
    };

    match key {
        Some(k) => ParseResult::Key(KeyEvent::new(k, modifier), consumed),
        None => ParseResult::Key(KeyEvent::new(Key::Escape, Modifiers::none()), 1),
    }
}

fn parse_ss3_sequence(bytes: &[u8]) -> ParseResult {
    if bytes.len() < 3 {
        return ParseResult::NeedMore;
    }

    let key = match bytes[2] {
        b'P' => Some(Key::F(1)),
        b'Q' => Some(Key::F(2)),
        b'R' => Some(Key::F(3)),
        b'S' => Some(Key::F(4)),
        b'H' => Some(Key::Home),
        b'F' => Some(Key::End),
        _ => None,
    };

    match key {
        Some(k) => ParseResult::Key(KeyEvent::new(k, Modifiers::none()), 3),
        None => ParseResult::Key(KeyEvent::new(Key::Escape, Modifiers::none()), 1),
    }
}

fn parse_modifier_param(s: &str) -> Modifiers {
    let n: u8 = s.parse().unwrap_or(1);
    // Modifier encoding: 1 + (shift ? 1 : 0) + (alt ? 2 : 0) + (ctrl ? 4 : 0)
    let mut m = Modifiers::none();
    let bits = n.saturating_sub(1);
    m.shift = (bits & 1) != 0;
    m.alt = (bits & 2) != 0;
    m.ctrl = (bits & 4) != 0;
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_regular_char() {
        let mut parser = KeyParser::new();
        parser.push(b"a");
        assert_eq!(
            parser.parse_next(),
            ParseResult::Key(KeyEvent::char('a'), 1)
        );
    }

    #[test]
    fn test_parse_ctrl_a() {
        let mut parser = KeyParser::new();
        parser.push(&[0x01]);
        assert_eq!(
            parser.parse_next(),
            ParseResult::Key(KeyEvent::ctrl_char('a'), 1)
        );
    }

    #[test]
    fn test_parse_arrow_up() {
        let mut parser = KeyParser::new();
        parser.push(b"\x1b[A");
        assert_eq!(
            parser.parse_next(),
            ParseResult::Key(KeyEvent::new(Key::Up, Modifiers::none()), 3)
        );
    }

    #[test]
    fn test_parse_f1() {
        let mut parser = KeyParser::new();
        parser.push(b"\x1bOP");
        assert_eq!(
            parser.parse_next(),
            ParseResult::Key(KeyEvent::new(Key::F(1), Modifiers::none()), 3)
        );
    }

    #[test]
    fn test_parse_alt_x() {
        let mut parser = KeyParser::new();
        parser.push(b"\x1bx");
        assert_eq!(
            parser.parse_next(),
            ParseResult::Key(KeyEvent::new(Key::Char('x'), Modifiers::alt()), 2)
        );
    }

    #[test]
    fn test_parse_escape_need_more() {
        let mut parser = KeyParser::new();
        parser.push(b"\x1b");
        assert_eq!(parser.parse_next(), ParseResult::NeedMore);
    }

    #[test]
    fn test_force_parse_escape() {
        let mut parser = KeyParser::new();
        parser.push(b"\x1b");
        assert_eq!(
            parser.force_parse_first(),
            Some(ParseResult::Key(
                KeyEvent::new(Key::Escape, Modifiers::none()),
                1
            ))
        );
    }
}
