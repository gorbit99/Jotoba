use std::{
    cmp::Ordering,
    collections::HashMap,
    fs::{self, File},
    io::{self, BufRead, BufReader},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime},
};

use ::japanese::JapaneseExt;
use actix_web::{
    rt::time::timeout,
    web::{self, Json},
};
use config::Config;
use error::api_error::RestError;
use itertools::Itertools;
use log::info;
use parse::jmdict::languages::Language;
use query_parser::QueryType;
use search::{
    query::{Query, QueryLang, UserSettings},
    query_parser,
    suggestions::{store_item, SuggestionSearch, TextSearch},
};
use serde::{Deserialize, Serialize};
use tokio_postgres::Client;
use utils::real_string_len;

/// Request struct for suggestion endpoint
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SuggestionRequest {
    pub input: String,
    #[serde(default)]
    pub lang: String,
}

/// Response struct for suggestion endpoint
#[derive(Clone, Debug, Serialize, Default)]
pub struct SuggestionResponse {
    pub suggestions: Vec<WordPair>,
}

/// a Word with kana and kanji if available
#[derive(Clone, Debug, Serialize, Default)]
pub struct WordPair {
    pub primary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<String>,
}

/// Max results to show
const MAX_RESULTS: i64 = 10;

#[derive(Clone, Debug)]
pub struct SuggestionItem {
    pub text: String,
    pub sequence: i32,
}

impl store_item::Item for SuggestionItem {
    fn get_text(&self) -> &str {
        &self.text
    }
}

/// In-memor storage for suggestions
static SUGGESTIONS: once_cell::sync::OnceCell<SuggestionSearch<Vec<SuggestionItem>>> =
    once_cell::sync::OnceCell::new();

/// Load Suggestions from suggestion folder
pub fn load_suggestions(config: &Config) {
    let mut map = HashMap::new();
    let path = config.get_suggestion_sources();

    if let Ok(entries) = fs::read_dir(path).and_then(|i| {
        i.map(|res| res.map(|e| e.path()))
            .collect::<Result<Vec<_>, io::Error>>()
    }) {
        for entry in entries {
            let entry_name = entry.file_name().unwrap().to_str().unwrap();
            let lang = Language::from_str(entry_name);
            if lang.is_err() {
                continue;
            }
            let suggestions = load_file(&entry);
            if let Some(suggestions) = suggestions {
                map.insert(lang.unwrap(), TextSearch::new(suggestions));
                info!("Loaded {:?} suggestion file", lang.unwrap());
            }
        }
    }

    SUGGESTIONS.set(SuggestionSearch::new(map)).ok();
}

/// Parse suggestion file
fn load_file(path: &PathBuf) -> Option<Vec<SuggestionItem>> {
    let file = File::open(path).ok()?;
    let content = BufReader::new(file)
        .lines()
        .map(|i| {
            i.ok().and_then(|i| {
                let mut split = i.split(',').rev();
                let number: i32 = split.next()?.parse().ok()?;
                let text: String = split.rev().join(",");
                Some(SuggestionItem {
                    text,
                    sequence: number,
                })
            })
        })
        .collect::<Option<Vec<SuggestionItem>>>()?;
    Some(content)
}

/// Get search suggestions
pub async fn suggestion(
    pool: web::Data<Arc<Client>>,
    config: web::Data<Config>,
    payload: Json<SuggestionRequest>,
) -> Result<Json<SuggestionResponse>, actix_web::Error> {
    let query_len = real_string_len(&payload.input);
    if query_len < 1 || query_len > 37 {
        return Err(RestError::BadRequest.into());
    }

    let start = SystemTime::now();

    let mut query_str = payload.input.as_str();

    // Some inputs place the roman letter of the japanese text while typing with romanized input.
    // If input is japanese but last character is a romanized letter, strip it off
    let last_char = query_str.chars().rev().next().unwrap();
    if query_parser::parse_language(query_str) == QueryLang::Japanese
        && last_char.is_roman_letter()
        && query_len > 1
    {
        query_str = &query_str[..query_str.bytes().count() - last_char.len_utf8()];
    }

    // Parse query
    let query = query_parser::QueryParser::new(
        query_str.to_owned(),
        QueryType::Words,
        UserSettings {
            user_lang: Language::from_str(&payload.lang).unwrap_or_default(),
            ..UserSettings::default()
        },
        0,
        0,
    )
    .parse()
    .ok_or(RestError::BadRequest)?;

    let result = timeout(
        Duration::from_millis(config.get_suggestion_timeout()),
        get_suggestion(&pool, query),
    )
    .await
    .map_err(|_| RestError::Timeout)??;

    println!("suggestion took: {:?}", start.elapsed());

    Ok(Json(result))
}

async fn get_suggestion(pool: &Client, query: Query) -> Result<SuggestionResponse, RestError> {
    let suggestions = get_suggestion_by_query(pool, &query).await?;

    if suggestions.suggestions.is_empty() && query.query.is_hiragana() {
        let new_query = Query {
            query: romaji::RomajiExt::to_katakana(query.query.as_str()),
            ..query.clone()
        };
        return Ok(get_suggestion_by_query(pool, &new_query).await?);
    }

    Ok(suggestions)
}

async fn get_suggestion_by_query(
    pool: &Client,
    query: &Query,
) -> Result<SuggestionResponse, RestError> {
    // Get sugesstions for matching language
    let mut word_pairs = match query.language {
        QueryLang::Japanese => japanese::suggestions(&pool, &query.query).await?,
        QueryLang::Foreign | QueryLang::Undetected => foreign::suggestions(&query, &query.query)
            .await
            .unwrap_or_default(),
    };

    // Put exact matches to top
    word_pairs.sort_by(|a, b| {
        let a_has_reading = a.has_reading(&query.query);
        let b_has_reading = b.has_reading(&query.query);

        if a_has_reading && !b_has_reading {
            Ordering::Less
        } else if b_has_reading && !a_has_reading {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    });

    Ok(SuggestionResponse {
        suggestions: word_pairs,
    })
}

mod japanese {
    use std::iter::FromIterator;

    use futures::{stream::FuturesOrdered, TryStreamExt};
    use itertools::Itertools;

    use super::*;

    /// Get suggestions for foreign search input
    pub(super) async fn suggestions(
        client: &Client,
        query_str: &str,
    ) -> Result<Vec<WordPair>, RestError> {
        get_sequence_ids(client, &query_str).await
    }

    async fn get_sequence_ids(
        client: &Client,
        query_str: &str,
    ) -> Result<Vec<WordPair>, RestError> {
        let seq_query = "SELECT sequence FROM dict WHERE reading LIKE $1 ORDER BY jlpt_lvl DESC NULLS LAST, ARRAY_LENGTH(priorities,1) DESC NULLS LAST, LENGTH(reading) LIMIT $2";

        let rows = client
            .query(
                seq_query,
                &[&format!("{}%", query_str).as_str(), &MAX_RESULTS],
            )
            .await?;

        let mut sequences: Vec<i32> = rows.into_iter().map(|i| i.get(0)).collect();
        sequences.dedup();

        Ok(load_words(&client, &sequences).await?)
    }

    async fn load_words(client: &Client, sequences: &[i32]) -> Result<Vec<WordPair>, RestError> {
        let word_query =
            "select reading, kanji from dict where sequence = $1 and (is_main or kanji = false)";

        let prepared = client.prepare(word_query).await?;

        Ok(FuturesOrdered::from_iter(sequences.into_iter().map(|i| {
            let cloned = prepared.clone();
            async move { client.query(&cloned, &[&i]).await }
        }))
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .filter_map(|word| {
            let words: Vec<(String, bool)> =
                word.into_iter().map(|i| (i.get(0), i.get(1))).collect_vec();

            let kana = words.iter().find(|i| !i.1)?.0.to_owned();
            let kanji = words.iter().find(|i| i.1).map(|i| i.0.to_owned());

            Some(WordPair {
                primary: kana,
                secondary: kanji,
            })
        })
        .collect())
    }
}

mod foreign {
    use super::*;

    pub async fn suggestions(query: &Query, query_str: &str) -> Option<Vec<WordPair>> {
        let lang = query.settings.user_lang;

        let res = SUGGESTIONS
            .get()?
            .search(query_str, lang)
            .await?
            .into_iter()
            .map(|i| WordPair {
                primary: i.text.to_owned(),
                secondary: None,
            })
            .take(10)
            .collect();

        Some(res)
    }
}

impl WordPair {
    /// Returns true if [`self`] contains [`reading`]
    fn has_reading(&self, reading: &str) -> bool {
        self.primary == reading
            || self
                .secondary
                .as_ref()
                .map(|i| i == reading)
                .unwrap_or_default()
    }
}
