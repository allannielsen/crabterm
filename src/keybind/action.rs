use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    Send(Vec<u8>),
    ToggleTimestamp,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Quit => write!(f, "quit"),
            Action::Send(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    write!(f, "send {:?}", s)
                } else {
                    write!(f, "send-bytes {:02x?}", bytes)
                }
            }
            Action::ToggleTimestamp => write!(f, "toggle-timestamp"),
        }
    }
}

/// Result of processing input through the keybind processor
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeybindResult {
    /// Pass these bytes through to the device
    Passthrough(Vec<u8>),
    /// Execute this action
    Action(Action),
    /// Input was consumed (e.g., prefix key pressed, waiting for more input)
    Consumed,
}
