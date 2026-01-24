use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Modifiers {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn ctrl() -> Self {
        Self {
            ctrl: true,
            ..Default::default()
        }
    }

    pub fn alt() -> Self {
        Self {
            alt: true,
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.ctrl && !self.alt && !self.shift
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        write!(f, "{}", parts.join("+"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    F(u8),
    Escape,
    Enter,
    Tab,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Char(c) => write!(f, "{}", c),
            Key::F(n) => write!(f, "F{}", n),
            Key::Escape => write!(f, "Escape"),
            Key::Enter => write!(f, "Enter"),
            Key::Tab => write!(f, "Tab"),
            Key::Backspace => write!(f, "Backspace"),
            Key::Up => write!(f, "Up"),
            Key::Down => write!(f, "Down"),
            Key::Left => write!(f, "Left"),
            Key::Right => write!(f, "Right"),
            Key::Home => write!(f, "Home"),
            Key::End => write!(f, "End"),
            Key::PageUp => write!(f, "PageUp"),
            Key::PageDown => write!(f, "PageDown"),
            Key::Insert => write!(f, "Insert"),
            Key::Delete => write!(f, "Delete"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub key: Key,
    pub modifiers: Modifiers,
}

impl KeyEvent {
    pub fn new(key: Key, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }

    pub fn char(c: char) -> Self {
        Self {
            key: Key::Char(c),
            modifiers: Modifiers::none(),
        }
    }

    pub fn ctrl_char(c: char) -> Self {
        Self {
            key: Key::Char(c),
            modifiers: Modifiers::ctrl(),
        }
    }
}

impl fmt::Display for KeyEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.modifiers.is_empty() {
            write!(f, "{}+{}", self.modifiers, self.key)
        } else {
            write!(f, "{}", self.key)
        }
    }
}
