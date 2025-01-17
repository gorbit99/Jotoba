use itertools::Itertools;
use jp_utils::furigana::{self, reading_part_ref::ReadingPartRef};
use localization::{traits::Translatable, TranslationDict};
use search::executor::search_result::SearchResult;
use types::jotoba::{
    kanji::Kanji,
    language::{param::AsLangParam, Language},
    names::Name,
    words::{filter_languages, sense::Sense, Word},
};

use crate::unescaped::UnescapedString;

/// Returns a list of all collocations of a word
pub fn get_collocations(word: &Word, lang: impl AsLangParam) -> Vec<(String, String)> {
    if !word.has_collocations() {
        return vec![];
    }

    let word_storage = resources::get().words();

    let mut words = word
        .collocations
        .as_ref()
        .unwrap()
        .iter()
        .filter_map(|i| word_storage.by_sequence(*i))
        .cloned()
        .collect::<Vec<_>>();

    filter_languages(words.iter_mut(), lang);

    words
        .into_iter()
        .map(|word| {
            let senses: Vec<String> = word
                .get_senses_with_en()
                .into_iter()
                .flatten()
                .take(5)
                .map(|i| i.glosses)
                .flatten()
                .map(|i| i.gloss)
                .collect();

            let reading = word.reading.kanji.unwrap_or(word.reading.kana).reading;

            (reading, senses.join(", "))
        })
        .collect()
}

#[inline]
pub fn unescaped_string<T: ToString>(s: T) -> UnescapedString {
    UnescapedString::new(s)
}

/// Returns the transive version of `word`
#[inline]
pub fn get_transitive_counterpart(word: &Word) -> Option<Word> {
    let seq_id = word.transive_version.as_ref()?.get();
    resources::get().words().by_sequence(seq_id).cloned()
}

/// Returns the intransive version of `word`
#[inline]
pub fn get_intransitive_counterpart(word: &Word) -> Option<Word> {
    let seq_id = word.intransive_version.as_ref()?.get();
    resources::get().words().by_sequence(seq_id).cloned()
}

/// Returns an example sentences of a `sense` if existing.
/// tries to use a sentence written in `language` or falls back to english
pub fn ext_sentence(
    sense: &Sense,
    language: &Language,
) -> Option<(Vec<ReadingPartRef<'static>>, &'static str)> {
    let sentence = resources::get()
        .sentences()
        .by_id(sense.example_sentence?)?;

    let translation = sentence
        .translation_for(*language)
        .or_else(|| sentence.translation_for(Language::English))?;

    let furigana = furigana::parse::unchecked(&sentence.furigana);
    Some((furigana, translation))
}

pub fn get_types_humanized(
    name: &Name,
    dict: &TranslationDict,
    lang: localization::language::Language,
) -> String {
    if let Some(ref n_types) = name.name_type {
        n_types
            .iter()
            .filter_map(|i| (!i.is_gender()).then(|| i.pgettext(dict, "name_type", Some(lang))))
            .join(", ")
    } else {
        String::from("")
    }
}

pub fn word_kanji<O>(res: &SearchResult<Word, O>) -> Vec<Kanji> {
    search::word::kanji::load_word_kanji_info(&res.items)
}

pub fn has_kanji<O>(res: &SearchResult<Word, O>) -> bool {
    !word_kanji(res).is_empty()
}
