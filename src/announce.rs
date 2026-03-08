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
