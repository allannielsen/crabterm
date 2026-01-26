use std::collections::HashMap;

use super::IoFilter;
use crate::keybind::config::SettingValue;

pub const NAME: &str = "charmap";
pub const SETTING_IMAP: &str = "charmap-imap";
pub const SETTING_OMAP: &str = "charmap-omap";

#[derive(Debug, Clone, Copy)]
enum Mapping {
    CrLf,    // \r -> \n
    CrCrLf,  // \r -> \r\n
    IgnCr,   // \r -> (nothing)
    LfCr,    // \n -> \r
    LfCrLf,  // \n -> \r\n
    IgnLf,   // \n -> (nothing)
    BsDel,   // 0x08 -> 0x7f
    DelBs,   // 0x7f -> 0x08
}

impl Mapping {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "crlf" => Some(Mapping::CrLf),
            "crcrlf" => Some(Mapping::CrCrLf),
            "igncr" => Some(Mapping::IgnCr),
            "lfcr" => Some(Mapping::LfCr),
            "lfcrlf" => Some(Mapping::LfCrLf),
            "ignlf" => Some(Mapping::IgnLf),
            "bsdel" => Some(Mapping::BsDel),
            "delbs" => Some(Mapping::DelBs),
            _ => None,
        }
    }

    fn apply(&self, byte: u8, output: &mut Vec<u8>) -> bool {
        match self {
            Mapping::CrLf if byte == b'\r' => {
                output.push(b'\n');
                true
            }
            Mapping::CrCrLf if byte == b'\r' => {
                output.push(b'\r');
                output.push(b'\n');
                true
            }
            Mapping::IgnCr if byte == b'\r' => true,
            Mapping::LfCr if byte == b'\n' => {
                output.push(b'\r');
                true
            }
            Mapping::LfCrLf if byte == b'\n' => {
                output.push(b'\r');
                output.push(b'\n');
                true
            }
            Mapping::IgnLf if byte == b'\n' => true,
            Mapping::BsDel if byte == 0x08 => {
                output.push(0x7f);
                true
            }
            Mapping::DelBs if byte == 0x7f => {
                output.push(0x08);
                true
            }
            _ => false,
        }
    }
}

pub struct CharmapFilter {
    enabled: bool,
    imap: Vec<Mapping>, // device -> terminal (filter_out)
    omap: Vec<Mapping>, // terminal -> device (filter_in)
}

impl CharmapFilter {
    pub fn new() -> Self {
        CharmapFilter {
            enabled: false,
            imap: Vec::new(),
            omap: Vec::new(),
        }
    }

    pub fn configure(&mut self, settings: &HashMap<String, SettingValue>) {
        if let Some(value) = settings.get(SETTING_IMAP).and_then(|v| v.as_str()) {
            self.imap = Self::parse_mappings(value);
            // Auto-enable if mappings are configured
            if !self.imap.is_empty() {
                self.enabled = true;
            }
        }
        if let Some(value) = settings.get(SETTING_OMAP).and_then(|v| v.as_str()) {
            self.omap = Self::parse_mappings(value);
            // Auto-enable if mappings are configured
            if !self.omap.is_empty() {
                self.enabled = true;
            }
        }
    }

    fn parse_mappings(value: &str) -> Vec<Mapping> {
        value
            .split(',')
            .filter_map(|s| Mapping::from_str(s.trim()))
            .collect()
    }

    fn apply_mappings(mappings: &[Mapping], buf: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(buf.len());
        for &byte in buf {
            let mut handled = false;
            for mapping in mappings {
                if mapping.apply(byte, &mut output) {
                    handled = true;
                    break;
                }
            }
            if !handled {
                output.push(byte);
            }
        }
        output
    }
}

impl Default for CharmapFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl IoFilter for CharmapFilter {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    fn filter_out(&mut self, buf: &[u8]) -> Vec<u8> {
        Self::apply_mappings(&self.imap, buf)
    }

    fn filter_in(&mut self, buf: &[u8]) -> Vec<u8> {
        Self::apply_mappings(&self.omap, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crlf_mapping() {
        let mappings = vec![Mapping::CrLf];
        assert_eq!(
            CharmapFilter::apply_mappings(&mappings, b"hello\r\nworld"),
            b"hello\n\nworld"
        );
    }

    #[test]
    fn test_lfcrlf_mapping() {
        let mappings = vec![Mapping::LfCrLf];
        assert_eq!(
            CharmapFilter::apply_mappings(&mappings, b"hello\nworld"),
            b"hello\r\nworld"
        );
    }

    #[test]
    fn test_delbs_mapping() {
        let mappings = vec![Mapping::DelBs];
        assert_eq!(
            CharmapFilter::apply_mappings(&mappings, b"hello\x7fworld"),
            b"hello\x08world"
        );
    }

    #[test]
    fn test_bsdel_mapping() {
        let mappings = vec![Mapping::BsDel];
        assert_eq!(
            CharmapFilter::apply_mappings(&mappings, b"hello\x08world"),
            b"hello\x7fworld"
        );
    }

    #[test]
    fn test_igncr_mapping() {
        let mappings = vec![Mapping::IgnCr];
        assert_eq!(
            CharmapFilter::apply_mappings(&mappings, b"hello\r\nworld"),
            b"hello\nworld"
        );
    }

    #[test]
    fn test_ignlf_mapping() {
        let mappings = vec![Mapping::IgnLf];
        assert_eq!(
            CharmapFilter::apply_mappings(&mappings, b"hello\r\nworld"),
            b"hello\rworld"
        );
    }

    #[test]
    fn test_multiple_mappings() {
        let mappings = vec![Mapping::CrLf, Mapping::DelBs];
        assert_eq!(
            CharmapFilter::apply_mappings(&mappings, b"hello\r\x7fworld"),
            b"hello\n\x08world" // CrLf maps \r->\n, DelBs maps \x7f->\x08
        );
    }

    #[test]
    fn test_parse_mappings() {
        let mappings = CharmapFilter::parse_mappings("crlf,delbs");
        assert_eq!(mappings.len(), 2);
    }

    #[test]
    fn test_configure() {
        let mut filter = CharmapFilter::new();
        let mut settings = HashMap::new();
        settings.insert(
            SETTING_IMAP.to_string(),
            SettingValue::String("crlf,delbs".to_string()),
        );
        settings.insert(
            SETTING_OMAP.to_string(),
            SettingValue::String("lfcrlf".to_string()),
        );
        filter.configure(&settings);

        assert!(filter.enabled());
        assert_eq!(filter.imap.len(), 2);
        assert_eq!(filter.omap.len(), 1);
    }
}
