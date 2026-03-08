pub fn expand_template(template: &str, source: &str, msg: &str) -> String {
    let now = chrono::Local::now();
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
                    expanded.push_str(msg);
                    chars.next();
                }
                Some('s') => {
                    expanded.push_str(source);
                    chars.next();
                }
                Some('t') => {
                    expanded.push_str(&now.format("%H:%M:%S").to_string());
                    chars.next();
                }
                Some('d') => {
                    expanded.push_str(&now.format("%Y-%m-%d").to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_template_simple() {
        let template = "MSG-%s: %m";
        let source = "Local";
        let msg = "Hello world";
        let expanded = expand_template(template, source, msg);
        assert!(expanded.starts_with("MSG-Local: Hello world"));
    }

    #[test]
    fn test_expand_template_with_time() {
        let template = "MSG-%s: %t %m";
        let source = "127.0.0.1:4000";
        let msg = "Connected";
        let expanded = expand_template(template, source, msg);
        // MSG-127.0.0.1:4000: 14:55:01 Connected
        assert!(expanded.contains("MSG-127.0.0.1:4000: "));
        assert!(expanded.contains(" Connected"));
        // Check for HH:MM:SS format (at least 2 colons in the time part)
        // Total colons should be 1 (in source) + 1 (after source) + 2 (in time) = 4
        assert!(expanded.chars().filter(|&c| c == ':').count() >= 4);
    }

    #[test]
    fn test_expand_template_percent() {
        let template = "%% %m";
        let source = "Local";
        let msg = "Message";
        let expanded = expand_template(template, source, msg);
        assert_eq!(expanded, "% Message");
    }

    #[test]
    fn test_expand_template_multiple() {
        let template = "%s %s %m %m";
        let source = "S";
        let msg = "M";
        let expanded = expand_template(template, source, msg);
        assert_eq!(expanded, "S S M M");
    }
}
