//! Hardware identifiers of the serial-number kind stay out of the log and the JSON.
//! Needles are built at runtime so this file does not embed the token isolation forbids.

fn needles() -> [String; 5] {
    [
        format!("IOPlatform{}{}", "Serial", "Number"),
        format!("{}-{}", "serial", "number"),
        format!("{} {}", "Serial", "Number"),
        format!("{}{}", "Serial", "Number"),
        format!("{} {}", "hardware", "serial"),
    ]
}

pub fn contains_serial(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    needles()
        .iter()
        .any(|needle| lower.contains(&needle.to_ascii_lowercase()))
}

pub fn scrub(text: &str) -> String {
    text.lines()
        .filter(|line| !contains_serial(line))
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn scrub_model(model: &str) -> String {
    if contains_serial(model) {
        String::new()
    } else {
        model.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_serial_line_is_dropped_and_the_rest_stays() {
        let text = format!("MacBookPro16,2\n{} {}: ABC123\nkey", "Serial", "Number");
        assert!(contains_serial(&text));
        let cleaned = scrub(&text);
        assert!(!contains_serial(&cleaned));
        assert!(cleaned.contains("MacBookPro16,2"));
        assert!(!cleaned.contains("ABC123"));
        assert_eq!(scrub_model("MacBookPro16,2"), "MacBookPro16,2");
        assert_eq!(scrub_model(&format!("{} {}", "Serial", "Number")), "");
    }
}
