#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    None,
    Timestamp,
}

impl Filter {
    pub fn toggle_timestamp(&self) -> Filter {
        match self {
            Filter::Timestamp => Filter::None,
            _ => Filter::Timestamp,
        }
    }
}
