pub mod dict;
pub mod inflection;
pub mod sense;

pub use dict::Dict;
use itertools::Itertools;
pub use sense::{Gloss, Sense};
use utils::to_option;

use crate::parse::jmdict::{languages::Language, part_of_speech::PartOfSpeech, priority::Priority};
use japanese::{
    accent::{AccentChar, Border},
    furigana::{self, SentencePartRef},
    JapaneseExt,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

use self::inflection::Inflections;

/// A single word item
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Word {
    pub sequence: u32,
    pub priorities: Option<Vec<Priority>>,
    pub reading: Reading,
    pub senses: Vec<Sense>,
    pub accents: Option<Vec<u8>>,
    pub furigana: Option<String>,
    pub jlpt_lvl: Option<u8>,
    pub collocations: Option<Vec<(String, String)>>,
}

/// Various readings of a word
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Reading {
    pub kana: Dict,
    pub kanji: Option<Dict>,
    pub alternative: Vec<Dict>,
}

impl Word {
    /// Returns true if a word is common
    #[inline]
    pub fn is_common(&self) -> bool {
        self.reading.get_reading().priorities.is_some()
    }

    /// Returns the jlpt level of a word. `None` if a word doesn't have a JLPT lvl assigned
    pub fn get_jlpt_lvl(&self) -> Option<u8> {
        self.jlpt_lvl
    }

    /// Returns the reading of a word
    #[inline]
    pub fn get_reading(&self) -> &Dict {
        self.reading.get_reading()
    }

    /// Return `true` if the word is a katakana word
    #[inline]
    pub fn is_katakana_word(&self) -> bool {
        self.reading.is_katakana()
    }

    /// Return all senses of a language
    #[inline]
    pub fn senses_by_lang(&self, language: Language) -> Option<Vec<Sense>> {
        let senses = self
            .senses
            .iter()
            .filter(|i| i.language == language)
            .cloned()
            .collect();

        to_option(senses)
    }

    /// Get senses ordered by language (non-english first)
    pub fn get_senses_orderd(&self, english_on_top: bool, _language: Language) -> Vec<Vec<Sense>> {
        let (english, other): (Vec<Sense>, Vec<Sense>) = self
            .senses
            .clone()
            .into_iter()
            .partition(|i| i.language == Language::English);

        if english_on_top {
            vec![english, other]
        } else {
            vec![other, english]
        }
    }

    /// Get senses ordered by language (non-english first)
    pub fn get_senses(&self) -> Vec<Vec<Sense>> {
        let (english, other): (Vec<Sense>, Vec<Sense>) = self
            .senses
            .clone()
            .into_iter()
            .partition(|i| i.language == Language::English);

        vec![other, english]
    }

    /// Get amount of tags which will be displayed below the reading
    #[inline]
    pub fn get_word_tag_count(&self) -> u8 {
        [self.is_common(), self.get_jlpt_lvl().is_some()]
            .iter()
            .filter(|b| **b)
            .count() as u8
    }

    /// Get the audio path of a word
    #[inline]
    pub fn audio_file(&self) -> Option<String> {
        self.reading.kanji.as_ref().and_then(|kanji| {
            let file = format!("{}【{}】.ogg", kanji.reading, self.reading.kana.reading);
            Path::new(&format!("html/assets/audio/{}", file))
                .exists()
                .then(|| file)
        })
    }

    /// Returns a renderable vec of accents with kana characters
    pub fn get_accents(&self) -> Option<Vec<AccentChar>> {
        let accents_raw = self.accents.as_ref()?;
        let kana = &self.reading.kana;
        let accents = japanese::accent::calc_pitch(&kana.reading, accents_raw[0] as i32)?;
        let accent_iter = accents.iter().peekable().enumerate();

        let res = accent_iter
            .map(|(pos, (part, is_high))| {
                let borders = vec![if *is_high {
                    Border::Top
                } else {
                    Border::Bottom
                }];
                let borders = if pos != accents.len() - 1 {
                    borders.into_iter().chain(vec![Border::Right]).collect()
                } else {
                    borders
                };
                vec![AccentChar { borders, c: part }]
            })
            .flatten()
            .into_iter()
            .collect();

        Some(res)
    }

    /// Returns furigana reading-pairs of an Item
    pub fn get_furigana(&self) -> Option<Vec<SentencePartRef<'_>>> {
        let furi = self.furigana.as_ref()?;
        Some(furigana::from_str(furi).collect::<Vec<_>>())
    }

    /// Get alternative readings in a beautified, print-ready format
    pub fn alt_readings_beautified(&self) -> String {
        self.reading
            .alternative
            .iter()
            .map(|i| i.reading.clone())
            .join(", ")
    }

    /// Returns an [`Inflections`] value if [`self`] is a valid verb
    pub fn get_inflections(&self) -> Option<Inflections> {
        inflection::of_word(self)
    }

    pub fn glosses_pretty(&self) -> String {
        let senses = self.get_senses();

        // Try to use glosses with users language
        if !senses[0].is_empty() {
            Self::pretty_print_senses(&senses[0])
        } else {
            // Fallback use english gloses
            Self::pretty_print_senses(&senses[1])
        }
    }

    fn pretty_print_senses(senses: &[Sense]) -> String {
        senses
            .iter()
            .map(|i| i.glosses.clone())
            .flatten()
            .into_iter()
            .map(|i| i.gloss)
            .join(", ")
    }

    /// Returns an iterator over all parts of speech of a word
    #[inline]
    fn get_pos(&self) -> impl Iterator<Item = &PartOfSpeech> {
        self.senses
            .iter()
            .map(|i| i.part_of_speech.iter())
            .flatten()
    }
}

impl Reading {
    /// Return `true` if reading represents a katakana only word
    #[inline]
    pub fn is_katakana(&self) -> bool {
        self.kana.reading.is_katakana() && self.kanji.is_none()
    }

    /// Returns the preferred word-reading of a `Reading`
    #[inline]
    pub fn get_reading(&self) -> &Dict {
        self.kanji.as_ref().unwrap_or(&self.kana)
    }
}
