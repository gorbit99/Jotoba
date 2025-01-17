use japanese::ToKanaExt;
use jp_utils::furigana::{self, as_part::AsPart};
use sentence_reader::JA_NL_PARSER;
use types::jotoba::{
    kanji::reading::{Reading, ReadingSearch},
    sentences::Sentence,
};

pub(crate) fn sentence_matches(sentence: &Sentence, reading: &Reading) -> bool {
    let lit = reading.get_lit_str();

    if reading.is_full_reading() {
        let parsed_furi = furigana::parse::from_str(&sentence.furigana);
        let reading_hira = reading.get_raw().to_hiragana();

        for i in parsed_furi {
            let i = i.unwrap();
            if !i.is_kanji() {
                continue;
            }

            let curr_kanji = i.as_kanji().unwrap();
            if !curr_kanji.contains(&lit) {
                continue;
            }

            if i.kana_reading().to_hiragana().contains(&reading_hira) {
                return true;
            }
        }

        return false;
    }

    // Kunyomi

    let formatted = reading.format_reading_with_literal();
    for morph in JA_NL_PARSER.get().unwrap().parse(&sentence.japanese) {
        let reading = morph.lexeme;
        if reading == formatted {
            return true;
        }
    }

    false
}

pub(crate) fn get_reading(reading: &ReadingSearch) -> Option<Reading> {
    let kanji_storage = resources::get().kanji();
    let kanji = kanji_storage.by_literal(reading.literal)?;
    let reading = kanji.find_reading(&reading.reading)?;
    Some(reading)
}
