//! Text from a file or a server is data. These decide how it may be shown, and spot a name built
//! to pass for another. The checks are a short heuristic, not UTS #39: they will miss names and flag
//! honest ones, and finding nothing never means a file is safe.

use std::sync::OnceLock;

use regex::Regex;

/// The characters the Config schema's `safeText` refuses: control, line-separator, invisible and
/// bidi characters.
pub fn is_hidden(c: char) -> bool {
    matches!(c,
        '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{ad}' | '\u{34f}' | '\u{61c}'
        | '\u{115f}' | '\u{1160}' | '\u{180e}' | '\u{200b}'..='\u{200f}'
        | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{3164}' | '\u{feff}'
        | '\u{ffa0}' | '\u{fff9}'..='\u{fffb}' | '\u{e0000}'..='\u{e007f}')
}

fn code_point(c: char) -> String {
    let n = u32::from(c);
    if n <= 0xffff {
        format!("\\u{n:04x}")
    } else {
        format!("\\u{{{n:x}}}")
    }
}

/// The text with anything outside printable ASCII written as a code point, so it can't pass as
/// something else.
pub fn escaped(text: &str) -> String {
    text.chars()
        .map(|c| {
            if (' '..='~').contains(&c) {
                c.to_string()
            } else {
                code_point(c)
            }
        })
        .collect()
}

/// As [`escaped`], keeping line breaks and tabs, for showing a whole script or a diff.
pub fn escaped_lines(text: &str) -> String {
    text.chars()
        .map(|c| {
            if (' '..='~').contains(&c) || c == '\n' || c == '\t' {
                c.to_string()
            } else {
                code_point(c)
            }
        })
        .collect()
}

/// A script's text as it can be shown, or `None` if it can't be: a file that isn't UTF-8 can't be
/// read by a person, so it can't be approved.
pub fn showable_script(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes).ok().map(escaped_lines)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Latin,
    Cyrillic,
    Greek,
    Fullwidth,
}

fn letter_script(c: char) -> Option<Script> {
    match c {
        'A'..='Z' | 'a'..='z' | '\u{c0}'..='\u{24f}' | '\u{1e00}'..='\u{1eff}' => {
            Some(Script::Latin)
        }
        '\u{370}'..='\u{3ff}' | '\u{1f00}'..='\u{1fff}' => Some(Script::Greek),
        '\u{400}'..='\u{52f}'
        | '\u{1c80}'..='\u{1c8f}'
        | '\u{2de0}'..='\u{2dff}'
        | '\u{a640}'..='\u{a69f}' => Some(Script::Cyrillic),
        '\u{ff21}'..='\u{ff3a}' | '\u{ff41}'..='\u{ff5a}' => Some(Script::Fullwidth),
        _ => None,
    }
}

/// Letters from another alphabet in text that also has Latin ones.
pub fn lookalikes(text: &str) -> Vec<char> {
    let scripts: Vec<Script> = text.chars().filter_map(letter_script).collect();
    if !scripts.contains(&Script::Latin) || scripts.iter().all(|s| *s == Script::Latin) {
        return Vec::new();
    }
    text.chars()
        .filter(|c| letter_script(*c).is_some_and(|s| s != Script::Latin))
        .collect()
}

/// A word in a name that claims someone checked this file, if there is one.
pub fn claim(name: &str) -> Option<String> {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| {
        Regex::new(r"(?i)\b(soulcaster|verified|official|certified|endorsed|approved|trusted)\b")
            .expect("a fixed pattern compiles")
    })
    .find(name)
    .map(|found| found.as_str().to_string())
}

/// A rough confusables folding, so `homebrevv` and `s0ulcaster` read as what they imitate.
pub fn skeleton(text: &str) -> String {
    let mut s = text.to_lowercase();
    for (from, to) in [("vv", "w"), ("rn", "m"), ("0", "o"), ("1", "l"), ("5", "s")] {
        s = s.replace(from, to);
    }
    s
}

/// The name this one imitates, if any.
pub fn fakes_a_name(owner: &str) -> Option<&'static str> {
    let folded = skeleton(owner);
    ["homebrew", "github", "apple", "soulcaster"]
        .into_iter()
        .find(|name| folded.contains(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_leaves_nothing_but_printable_ascii() {
        assert_eq!(
            escaped("a\u{202e}b\n\u{1f600}"),
            "a\\u202eb\\u000a\\u{1f600}"
        );
        assert_eq!(escaped_lines("a\n\u{1b}[2K\tb"), "a\n\\u001b[2K\tb");
    }

    #[test]
    fn a_script_that_is_not_utf8_cannot_be_shown() {
        assert_eq!(showable_script(&[0xff, 0xfe]), None);
        assert_eq!(showable_script(b"echo hi\n").as_deref(), Some("echo hi\n"));
    }

    #[test]
    fn mixed_alphabets_are_found_and_one_alphabet_is_not() {
        assert_eq!(lookalikes("\u{410}pple"), ['\u{410}']);
        assert_eq!(lookalikes("\u{ff41}pple"), ['\u{ff41}']);
        assert!(lookalikes("Apple").is_empty());
        assert!(lookalikes("\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}").is_empty());
    }

    #[test]
    fn folding_catches_the_usual_imitations() {
        assert_eq!(fakes_a_name("homebrevv"), Some("homebrew"));
        assert_eq!(fakes_a_name("s0ulcaster-official"), Some("soulcaster"));
        assert_eq!(fakes_a_name("GITHUB-tools"), Some("github"));
        assert_eq!(fakes_a_name("example-dev"), None);
        assert_eq!(claim("Totally Verified!").as_deref(), Some("Verified"));
        assert_eq!(claim("unverifiedish"), None);
    }
}
