/// Escape provider-controlled text before placing it in terminal headings or tables.
///
/// JSON serialization retains the original value; this function is only for human terminal
/// chrome where controls and bidi overrides could obscure the selected resource.
pub(crate) fn terminal_safe(value: &str) -> String {
    value.chars().fold(String::new(), |mut output, character| {
        if character.is_control() || is_bidi_control(character) {
            output.extend(character.escape_unicode());
        } else {
            output.push(character);
        }
        output
    })
}

/// Escape terminal-control data from captured remote output while retaining line structure.
pub(crate) fn terminal_output_safe(value: &str) -> String {
    value.chars().fold(String::new(), |mut output, character| {
        if matches!(character, '\n' | '\t') {
            output.push(character);
        } else if character.is_control() || is_bidi_control(character) {
            output.extend(character.escape_unicode());
        } else {
            output.push(character);
        }
        output
    })
}

const fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(test)]
mod tests {
    use super::{terminal_output_safe, terminal_safe};

    #[test]
    fn terminal_text_escapes_controls_and_bidi_overrides() {
        assert_eq!(
            terminal_safe("site\nname\u{202e}txt"),
            "site\\u{a}name\\u{202e}txt"
        );
    }

    #[test]
    fn captured_output_keeps_lines_but_escapes_terminal_sequences() {
        assert_eq!(
            terminal_output_safe("first\nsecond\u{1b}[31m"),
            "first\nsecond\\u{1b}[31m"
        );
    }
}
