use crate::query::Tag;
use once_cell::sync::Lazy;
use regex::Regex;
use std::str::FromStr;
use types::jotoba::{
    search::SearchTarget,
    words::{misc::Misc, part_of_speech::PosSimple},
};
use utils::trim_string_end;

/// Regex for finding tags within a query.
static TAG_REGEX: Lazy<Regex> = Lazy::new(|| regex::Regex::new("#[a-zA-Z0-9\\-]*").unwrap());

/// Extracts all tags from the query and returns a new one without tags along with those tags which were extracted
pub fn extract_parse<'a, F>(inp: &'a str, parse: F) -> (String, Vec<Tag>)
where
    F: Fn(&str) -> (Option<Tag>, bool),
{
    let mut new_out = inp.to_string();

    let mut tags = vec![];

    // We edit the string so we have to keep track of how many bytes
    // we already removed in order to remove the correct range from the string
    let mut delta = 0;
    for m in TAG_REGEX.find_iter(inp) {
        let tag_str = m.as_str();

        let (tag, remove) = parse(tag_str);

        if let Some(tag) = tag {
            tags.push(tag);
        }

        if !remove {
            continue;
        }

        // Remove tag-str from query
        let r = m.range();
        let s = r.start - delta;
        let mut e = r.end - delta;

        // Strip space from tag too
        if new_out.len() > e + 1 && inp.is_char_boundary(e + 1) && &inp[e..e + 1] == " " {
            e += 1;
            delta += 1;
        }
        new_out.replace_range(s..e, "");
        delta += r.len();
    }

    (trim_string_end(new_out), tags)
}

/// Parse a tag from a string
pub fn parse(s: &str) -> Option<Tag> {
    if let Some(tag) = s.to_lowercase().strip_prefix("#") {
        match tag {
            "hidden" | "hide" => return Some(Tag::Hidden),
            "irrichidan" | "irregularichidan" | "irregular-ichidan" => {
                return Some(Tag::IrregularIruEru)
            }
            _ => (),
        }
    }

    #[allow(irrefutable_let_patterns)]
    if let Some(tag) = parse_genki_tag(s) {
        return Some(tag);
    } else if let Some(tag) = parse_jlpt_tag(s) {
        return Some(tag);
    } else if let Some(tag) = parse_search_type(s) {
        return Some(tag);
    } else {
        match PosSimple::from_str(&s[1..]) {
            Ok(pos) => return Some(Tag::PartOfSpeech(pos)),
            _ => return None,
        }
    }
}

/// Returns `Some(u8)` if `s` is a valid N-tag
fn parse_jlpt_tag(s: &str) -> Option<Tag> {
    if s.chars().skip(1).next()?.to_lowercase().next()? != 'n' {
        return None;
    }

    let nr: u8 = s[2..].parse().ok()?;
    (nr > 0 && nr < 6).then(|| Tag::Jlpt(nr))
}

/// Returns `Some(u8)` if `s` is a valid genki-tag
fn parse_genki_tag(s: &str) -> Option<Tag> {
    let e = s.trim().strip_prefix("#")?.trim().to_lowercase();
    if !e.starts_with("genki") {
        return None;
    }

    let nr: u8 = s[6..].parse().ok()?;
    (nr >= 3 && nr <= 23).then(|| Tag::GenkiLesson(nr))
}

/// Parse only search type
fn parse_search_type(s: &str) -> Option<Tag> {
    Some(match s[1..].to_lowercase().as_str() {
        "kanji" => Tag::SearchType(SearchTarget::Kanji),
        "sentence" | "sentences" => Tag::SearchType(SearchTarget::Sentences),
        "name" | "names" => Tag::SearchType(SearchTarget::Names),
        "word" | "words" => Tag::SearchType(SearchTarget::Words),
        "abbreviation" | "abbrev" => Tag::Misc(Misc::Abbreviation),
        _ => return None,
    })
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_jlpt_tag_parsing() {
        assert_eq!(parse_jlpt_tag("#n4"), Some(Tag::Jlpt(4)));
    }

    #[test]
    fn test_parse_genki_tag_parsing() {
        assert_eq!(parse_genki_tag("#genki3"), Some(Tag::GenkiLesson(3)));
        assert_eq!(parse_genki_tag("#genki23"), Some(Tag::GenkiLesson(23)));
    }
}