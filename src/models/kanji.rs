use super::{
    super::schema::{kanji, kanji_element},
    dict::{self, Dict},
    radical::{self, Radical},
};
use crate::{
    cache::SharedCache,
    error::Error,
    parse::{kanji_ele::KanjiPart, kanjidict::Character},
    search::{query::KanjiReading, SearchMode},
    utils::{self, invert_ordering, to_option},
    DbPool,
};

#[cfg(feature = "tokenizer")]
use crate::JA_NL_PARSER;

use async_std::sync::{Mutex, MutexGuard};
use diesel::{
    prelude::*,
    sql_types::{Bool, Text},
};
use itertools::Itertools;
use once_cell::sync::Lazy;
use romaji::RomajiExt;
use std::{cmp::Ordering, collections::HashMap};
use tokio_diesel::*;

/// An in memory Cache for kanji items
static KANJICACHE_C: Lazy<Mutex<SharedCache<i32, Kanji>>> =
    Lazy::new(|| Mutex::new(SharedCache::with_capacity(10000)));

#[derive(Queryable, QueryableByName, Clone, Debug, Default, PartialEq)]
#[table_name = "kanji"]
pub struct Kanji {
    pub id: i32,
    pub literal: String,
    pub meaning: Vec<String>,
    pub grade: Option<i32>,
    pub radical: Option<i32>,
    pub stroke_count: i32,
    pub frequency: Option<i32>,
    pub jlpt: Option<i32>,
    pub variant: Option<Vec<String>>,
    pub onyomi: Option<Vec<String>>,
    pub kunyomi: Option<Vec<String>>,
    pub chinese: Option<Vec<String>>,
    pub korean_r: Option<Vec<String>>,
    pub korean_h: Option<Vec<String>>,
    pub natori: Option<Vec<String>>,
    pub kun_dicts: Option<Vec<i32>>,
}

#[derive(Insertable, Clone, Debug, PartialEq, Default)]
#[table_name = "kanji"]
pub struct NewKanji {
    pub literal: String,
    pub meaning: Vec<String>,
    pub grade: Option<i32>,
    pub radical: Option<i32>,
    pub stroke_count: i32,
    pub frequency: Option<i32>,
    pub jlpt: Option<i32>,
    pub variant: Option<Vec<String>>,
    pub onyomi: Option<Vec<String>>,
    pub kunyomi: Option<Vec<String>>,
    pub chinese: Option<Vec<String>>,
    pub korean_r: Option<Vec<String>>,
    pub korean_h: Option<Vec<String>>,
    pub natori: Option<Vec<String>>,
    pub kun_dicts: Option<Vec<i32>>,
}

#[derive(Queryable, QueryableByName, Clone, Debug, Default, PartialEq)]
#[table_name = "kanji_element"]
pub struct KanjiElement {
    pub id: i32,
    pub kanji_id: i32,
    pub search_radical_id: i32,
}

#[derive(Insertable, Clone, Debug, Default, PartialEq)]
#[table_name = "kanji_element"]
pub struct NewKanjiElement {
    pub kanji_id: i32,
    pub search_radical_id: i32,
}

impl From<Character> for NewKanji {
    fn from(k: Character) -> Self {
        Self {
            literal: k.literal.into(),
            meaning: k.meaning.clone(),
            grade: k.grade,
            stroke_count: k.stroke_count,
            frequency: k.frequency,
            jlpt: k.jlpt,
            variant: to_option(k.variant),
            onyomi: to_option(k.on_readings),
            kunyomi: to_option(k.kun_readings),
            chinese: to_option(k.chinese_readings),
            korean_r: to_option(k.korean_romanized),
            korean_h: to_option(k.korean_hangul),
            natori: to_option(k.natori),
            kun_dicts: None,
            radical: k.radical,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReadingType {
    Kunyomi,
    Onyomi,
}

use futures::future::try_join_all;

pub async fn insert_kanji_part(db: &DbPool, element: KanjiPart) -> Result<(), Error> {
    // Find kanji
    let kanji = match find_by_literal(db, element.radical.to_string()).await {
        Ok(v) => v,
        Err(err) => match err {
            Error::DbError(diesel::result::Error::NotFound) => return Ok(()),
            _ => return Err(err),
        },
    };

    // Find search_radicals IDs for all parts
    let literals = try_join_all(
        element
            .parts
            .into_iter()
            .map(|i| radical::search_radical_find_by_literal(db, i)),
    )
    .await?;

    let new_kanji_elements = literals
        .into_iter()
        .map(|i| NewKanjiElement {
            search_radical_id: i.id,
            kanji_id: kanji.id,
        })
        .collect_vec();

    // Insert all search_radicals assigned to the kanji in kanji_elements table
    insert_kanji_elements(db, &new_kanji_elements).await?;

    Ok(())
}

async fn insert_kanji_elements(db: &DbPool, items: &[NewKanjiElement]) -> Result<(), Error> {
    diesel::insert_into(kanji_element::table)
        .values(items)
        .execute_async(db)
        .await?;
    Ok(())
}

impl Kanji {
    /// Returns all dict entries assigned to the kanji's kun readings
    pub async fn get_kun_readings(db: &DbPool, ids: &[i32]) -> Result<Vec<Dict>, Error> {
        dict::load_by_ids(db, ids).await
    }

    pub async fn load_radical(&self, db: &DbPool) -> Result<Radical, Error> {
        Ok(radical::find_by_id(db, self.radical.unwrap()).await?)
    }

    /// Print kanji grade pretty for frontend
    pub fn school_str(&self) -> Option<String> {
        self.grade.map(|grade| format!("Taught in {} grade", grade))
    }

    /// Returns the ReadingType of the reading for a kanji
    pub fn get_reading_type(&self, reading: &String) -> Option<ReadingType> {
        let in_on = self.in_on_reading(reading);
        let in_kun = self.in_kun_reading(reading);

        if in_on && !in_kun {
            return Some(ReadingType::Onyomi);
        } else if !in_on && in_kun {
            return Some(ReadingType::Kunyomi);
        }

        None
    }

    pub fn in_kun_reading(&self, reading: &String) -> bool {
        self.kunyomi
            .as_ref()
            .and_then(|j| j.contains(reading).then(|| true))
            .unwrap_or(false)
    }

    pub fn in_on_reading(&self, reading: &String) -> bool {
        self.onyomi
            .as_ref()
            .and_then(|j| j.contains(reading).then(|| true))
            .unwrap_or(false)
    }

    /// Returns true if kanji has a given reading
    pub fn has_reading(&self, reading: &String) -> bool {
        self.kunyomi
            .as_ref()
            .and_then(|i| i.contains(reading).then(|| true))
            .unwrap_or_else(|| {
                self.onyomi
                    .as_ref()
                    .and_then(|j| j.contains(reading).then(|| true))
                    .unwrap_or(false)
            })
    }

    /// Find sequence ids of dict entries containing
    /// the kanji and the passed reading
    pub async fn find_readings(
        &self,
        db: &DbPool,
        reading: &KanjiReading,
        r_type: ReadingType,
        mode: SearchMode,
        only_main_readins: bool,
    ) -> Result<Vec<i32>, Error> {
        let query = include_str!("../../sql/words_with_kanji_readings.sql");

        let lit_sql_hg = mode.to_like(format_reading(&reading.reading).to_hiragana());
        let lit_sql_kk = mode.to_like(format_reading(&reading.reading).to_katakana());
        let lit_reading = mode.to_like(self.format_reading(&reading.reading, r_type));

        /*
        println!(
            "{}",
            diesel::debug_query::<diesel::pg::Pg, _>(
                &diesel::sql_query(query)
                    .bind::<Text, _>(&lit_sql_hg)
                    .bind::<Text, _>(&lit_sql_kk)
                    .bind::<Text, _>(&lit_reading)
                    .bind::<Bool, _>(only_main_readins)
            )
            .to_string()
        );
        */

        let res = diesel::sql_query(query)
            .bind::<Text, _>(&lit_sql_hg)
            .bind::<Text, _>(&lit_sql_kk)
            .bind::<Text, _>(&lit_reading)
            .bind::<Bool, _>(only_main_readins)
            .get_results_async::<Dict>(db)
            .await?;

        Ok(res.into_iter().map(|i| i.sequence).collect())
    }

    pub fn format_reading(&self, reading: &str, r_type: ReadingType) -> String {
        match r_type {
            ReadingType::Kunyomi => {
                let r = if reading.contains('.') {
                    let right = reading.split('.').nth(1).unwrap_or_default();
                    format!("{}{}", self.literal, right)
                } else {
                    self.literal.to_string()
                };
                r.replace("-", "")
            }
            ReadingType::Onyomi => self.literal.clone(),
        }
    }
}

/// Formats a kun/on reading to a kana entry
pub fn format_reading(reading: &str) -> String {
    reading.replace('-', "").replace('.', "")
}

/// Update the jlpt information for a kanji by its literal
pub async fn update_jlpt(db: &DbPool, l: &str, level: i32) -> Result<(), Error> {
    use crate::schema::kanji::dsl::*;
    diesel::update(kanji)
        .filter(literal.eq(l))
        .set(jlpt.eq(level))
        .execute_async(db)
        .await?;
    Ok(())
}

/// Inserts a new kanji into db
pub async fn insert<T>(db: &DbPool, new_kanji: Vec<T>) -> Result<(), Error>
where
    T: Into<NewKanji>,
{
    use crate::schema::kanji::dsl::*;

    let items: Vec<NewKanji> = new_kanji.into_iter().map(|i| i.into()).collect();

    diesel::insert_into(kanji)
        .values(items)
        .execute_async(db)
        .await?;

    Ok(())
}

/// Clear all kanji entries
pub async fn clear_kanji_elements(db: &DbPool) -> Result<(), Error> {
    use crate::schema::kanji_element::dsl::*;
    diesel::delete(kanji_element).execute_async(db).await?;
    Ok(())
}

/// Clear all kanji entries
pub async fn clear_kanji(db: &DbPool) -> Result<(), Error> {
    use crate::schema::kanji::dsl::*;
    diesel::delete(kanji).execute_async(db).await?;
    Ok(())
}

/// Returns Ok(true) if at least one kanji exists in the Db
pub async fn exists(db: &DbPool) -> Result<bool, Error> {
    use crate::schema::kanji::dsl::*;
    Ok(kanji.select(id).limit(1).execute_async(db).await? == 1)
}

/// Find a kanji by its literal
pub async fn find_by_literal(db: &DbPool, l: String) -> Result<Kanji, Error> {
    // Try to find literal in kanji cache
    let mut k_cache: MutexGuard<SharedCache<i32, Kanji>> = KANJICACHE_C.lock().await;
    if let Some(k) = k_cache.find_by_predicate(|i| i.literal == l) {
        return Ok(k.clone());
    }

    let db_kanji = load_by_literal(db, &l).await?;

    // Add to cache for future usage
    k_cache.cache_set(db_kanji.id, db_kanji.clone());

    Ok(db_kanji)
}

/// Find a kanji by its literal
pub async fn find_by_literals(db: &DbPool, l: &[String]) -> Result<Vec<Kanji>, Error> {
    if l.is_empty() {
        return Ok(vec![]);
    }

    // Try to find literal in kanji cache
    let mut k_cache: MutexGuard<SharedCache<i32, Kanji>> = KANJICACHE_C.lock().await;

    // Get cached kanji
    let cached_kanji = k_cache.filter_values(|i| l.contains(&i.literal));

    // Filter all literals which are not cached
    let missing_literals = l
        .iter()
        .filter_map(|i| (!cached_kanji.iter().any(|j| j.literal == **i)).then(|| i))
        .collect_vec();

    let db_kanji = load_by_literals(db, &missing_literals).await?;

    // Add to cache for future usage
    k_cache.extend(db_kanji.clone(), |i| i.id);

    Ok(cached_kanji.into_iter().chain(db_kanji).collect_vec())
}

/// Find Kanji items by its ids
pub async fn load_by_ids(db: &DbPool, ids: &[i32]) -> Result<Vec<Kanji>, Error> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    // Lock cache
    let mut k_cache: MutexGuard<SharedCache<i32, Kanji>> = KANJICACHE_C.lock().await;

    // Get cached kanji
    let cached_kanji = k_cache.get_values(&ids);

    // Determine which of the kanji
    // still needs to get looked up
    let to_lookup = ids
        .iter()
        .filter(|k_id| !cached_kanji.iter().any(|ci| ci.id == **k_id))
        .copied()
        .collect::<Vec<_>>();

    let db_result = retrieve_by_ids(&db, &to_lookup).await?;

    // Add result to cache for next time
    k_cache.extend(db_result.clone(), |i| i.id);

    Ok([db_result, cached_kanji].concat())
}

/// Retrieve kanji by ids from DB
async fn retrieve_by_ids(db: &DbPool, ids: &[i32]) -> Result<Vec<Kanji>, Error> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    use crate::schema::kanji::dsl::*;
    Ok(kanji.filter(id.eq_any(ids)).get_results_async(db).await?)
}

/// Load a kanji by its literal from DB
async fn load_by_literals(db: &DbPool, l: &[&String]) -> Result<Vec<Kanji>, Error> {
    use crate::schema::kanji::dsl::*;

    if l.is_empty() {
        return Ok(vec![]);
    }

    Ok(kanji
        .filter(literal.eq_any(l))
        .get_results_async(db)
        .await?)
}

/// Load a kanji by its literal from DB
async fn load_by_literal(db: &DbPool, l: &str) -> Result<Kanji, Error> {
    use crate::schema::kanji::dsl::*;

    Ok(kanji
        .filter(literal.eq(l))
        .limit(1)
        .get_result_async(db)
        .await?)
}

/// Clear existinig kun links
async fn clear_kun_links(db: &DbPool) -> Result<(), Error> {
    use crate::schema::kanji::dsl::*;

    let empty: Vec<i32> = Vec::new();
    diesel::update(kanji)
        .set(kun_dicts.eq(&empty))
        .execute_async(&db)
        .await?;

    Ok(())
}

/// Update kun reading links
pub async fn update_kun_links(db: &DbPool) -> Result<(), Error> {
    use crate::schema::kanji::dsl::*;

    clear_kun_links(db).await?;

    let all_kanji: Vec<(i32, String, Option<Vec<String>>, Option<Vec<String>>)> = kanji
        .select((id, literal, kunyomi, onyomi))
        .get_results_async(db)
        .await?;

    let mut dict_cache: HashMap<i32, Dict> = HashMap::new();
    print!("Updating kun readings... 0%");

    let all_kuns = all_kanji
        .iter()
        .filter_map(|i| {
            (i.2.is_some() && !i.2.as_ref().unwrap().is_empty())
                .then(|| (i.0, &i.1, i.2.as_ref().unwrap(), &i.3))
        })
        .enumerate()
        .filter_map(|(pos, (kid, klit, kuns, _))| {
            // For every kanji in DB
            print!(
                "\rUpdating kun readings... {}%",
                pos * 100 / all_kanji.len()
            );
            utils::to_option(
                get_kun_by_literal(db, klit.clone(), &kuns, &mut dict_cache).unwrap_or_default(),
            )
            .map(|r| (kid, r))
        })
        .collect::<Vec<(i32, Vec<_>)>>();

    println!();

    for k in all_kuns.chunks(100).into_iter() {
        futures::future::try_join_all(
            k.iter()
                .map(|(k_id, dict_ids)| update_kun_link(db, *k_id, dict_ids)),
        )
        .await?;
    }

    Ok(())
}

pub async fn update_kun_link(db: &DbPool, kanji_id: i32, dict_ids: &[i32]) -> Result<(), Error> {
    use crate::schema::kanji::dsl::*;
    diesel::update(kanji)
        .filter(id.eq(kanji_id))
        .set(kun_dicts.eq(dict_ids))
        .execute_async(db)
        .await?;
    Ok(())
}

/// Returns all kun reading compounds for a kanji
/// given by its literal
pub fn get_kun_by_literal(
    db: &DbPool,
    literal: String,
    kun: &[String], // All kanji kun readings
    cache: &mut HashMap<i32, Dict>,
) -> Result<Vec<i32>, Error> {
    let db = db.get().unwrap();
    use crate::schema::dict::dsl::*;

    // Find all Dict-seq_ids starting with the literal
    let seq_ids: Vec<i32> = dict
        .select(sequence)
        .filter(reading.like(format!("{}%", literal)))
        .filter(kanji.eq(true))
        .get_results(&db)?;

    // Get precached
    let cached = seq_ids
        .iter()
        .filter_map(|i| cache.get(i).cloned())
        .collect_vec();

    let dicts: Vec<Dict> = dict
        .filter(
            sequence.eq_any(
                seq_ids
                    .iter()
                    .filter(|i| !cache.contains_key(i))
                    .collect_vec(),
            ),
        )
        .order_by(id)
        .get_results(&db)?;

    // add to cache
    for d in dicts.iter() {
        cache.insert(d.sequence, d.clone());
    }

    // Concat results + cached
    let dicts = dicts.into_iter().chain(cached).collect_vec();

    // result vec
    let mut kuns: Vec<Dict> = Vec::new();

    // Iterate over all dicts containing the literal
    for (_, val) in dicts.iter().group_by(|i| i.sequence).into_iter() {
        let (kanji_r, kana_r): (Vec<Dict>, Vec<Dict>) =
            val.into_iter().cloned().partition(|i| i.kanji);
        if kanji_r.is_empty() {
            continue;
        }
        // kana reading of curr dict
        let dict_kana = kana_r[0].clone();
        // kanji reading of curr dict
        let dict_kanji = kanji_r[0].clone();

        for ku in kun.iter() {
            if kun_matches_kanji(&literal, ku, &dict_kana.reading, &dict_kanji.reading)
                && kun_len(ku) <= dict_kana.len()
            {
                kuns.push(dict_kanji);
                break;
            }
        }
    }

    let clean_kuns = kun.iter().map(|i| kun_literal_reading(i)).collect_vec();
    if kuns.len() > 10 {
        kuns.sort_by(|a, b| {
            let a_kunr = clean_kuns.contains(&a.reading);
            let b_kunr = clean_kuns.contains(&b.reading);

            if a_kunr && b_kunr {
                if let Some(order) = utils::get_item_order(&clean_kuns, &a.reading, &b.reading) {
                    return order;
                }
            } else if a_kunr && !b_kunr {
                return Ordering::Less;
            } else if !a_kunr && b_kunr {
                return Ordering::Greater;
            }

            #[cfg(feature = "tokenizer")]
            let a_parsed = JA_NL_PARSER.parse(&a.reading).len();
            #[cfg(feature = "tokenizer")]
            let b_parsed = JA_NL_PARSER.parse(&b.reading).len();
            #[cfg(feature = "tokenizer")]
            if a_parsed == 1 && b_parsed > 0 {
                return Ordering::Less;
            } else if a_parsed > 1 && b_parsed == 0 {
                return Ordering::Greater;
            }

            let a_prio = a.priorities.as_ref().map(|i| i.len()).unwrap_or_default();
            let b_prio = b.priorities.as_ref().map(|i| i.len()).unwrap_or_default();
            if a_prio > 0 && b_prio == 0 {
                return Ordering::Less;
            } else if b_prio > 0 && a_prio == 0 {
                return Ordering::Greater;
            }

            let a_jlpt = a.jlpt_lvl;
            let b_jlpt = b.jlpt_lvl;

            if a_jlpt.is_some() && b_jlpt.is_none() {
                return Ordering::Less;
            } else if a_jlpt.is_none() && b_jlpt.is_some() {
                return Ordering::Greater;
            }

            if a_jlpt.is_some() && b_jlpt.is_some() {
                let a_jlpt = a_jlpt.unwrap();
                let b_jlpt = b_jlpt.unwrap();
                return invert_ordering(a_jlpt.cmp(&b_jlpt));
            }

            Ordering::Equal
        });
        kuns.truncate(10);
    }

    Ok(kuns.iter().map(|i| i.sequence).collect())
}

pub fn kun_len(kun: &str) -> usize {
    utils::real_string_len(&format_reading(kun))
}

pub fn kun_literal_reading(kun: &str) -> String {
    kun.replace('-', "").split('.').next().unwrap().to_string()
}

fn kun_matches_kanji(literal: &str, kun: &str, kana_reading: &str, kanji_reading: &str) -> bool {
    let match_mode = if kun.starts_with('-') {
        SearchMode::RightVariable
    } else if kun.ends_with('-') || kanji_reading.starts_with(&literal) {
        SearchMode::LeftVariable
    } else {
        SearchMode::Exact
    };

    let mut kanji_out = kun.to_string().replace('-', "");

    if kun.contains('.') {
        let kun_left = kun.split('.').next().unwrap();
        kanji_out = kanji_out.replace(&format!("{}.", kun_left), literal);
    } else {
        kanji_out = literal.to_owned();
    }

    let kanji_out = kanji_out.replace(literal, &kun_literal_reading(kun));
    match_mode.str_eq(kana_reading, kanji_out.as_str(), false)
}

/*
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_kun_to_kanji2() {
        let literal = "古";
        let kun = "ふる-";
        let reading = "ふるいえ";
        let kanji_reading = "古いいえ";
        assert!(kun_matches_kanji(literal, kun, reading, kanji_reading));
    }

    #[test]
    fn test_kun_to_kanji() {
        let literal = "疼";
        let kun = "うず.く";
        let reading = "うずく";
        let kanji_reading = "疼く";
        assert!(kun_matches_kanji(literal, kun, reading, kanji_reading));
    }

    #[test]
    fn test_kun_to_kanji3() {
        let literal = "新";
        let kun = "あら-";
        let reading = "しんあん";
        let kanji_reading = "新案";
        assert!(kun_matches_kanji(literal, kun, reading, kanji_reading));
    }

    #[test]
    fn test_kun_to_kanji4() {
        let literal = "新";
        let kun = "あたらしい";
        let reading = "あたらしい";
        let kanji_reading = "新しい";
        assert!(kun_matches_kanji(literal, kun, reading, kanji_reading));
    }
}
*/
