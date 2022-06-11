use crate::engine::{Indexable, SearchEngine};
use indexes::sentences::document::SentenceDocument;
use resources::storage::ResourceStorage;
use types::jotoba::{languages::Language, sentences::Sentence};
use vector_space_model2::{DefaultMetadata, Vector};

pub struct Engine {}

impl Indexable for Engine {
    type Metadata = DefaultMetadata;
    type Document = SentenceDocument;

    #[inline]
    fn get_index(
        _language: Option<Language>,
    ) -> Option<&'static vector_space_model2::Index<Self::Document, Self::Metadata>> {
        Some(indexes::get().sentence().native())
    }
}

impl SearchEngine for Engine {
    type Output = &'static Sentence;

    #[inline]
    fn doc_to_output(
        storage: &'static ResourceStorage,
        inp: &Self::Document,
    ) -> Option<Vec<Self::Output>> {
        storage.sentences().by_id(inp.seq_id).map(|i| vec![i])
    }

    fn gen_query_vector(
        index: &vector_space_model2::Index<Self::Document, Self::Metadata>,
        query: &str,
        _allow_align: bool,
        _language: Option<Language>,
    ) -> Option<(Vector, String)> {
        let mut terms = vec![query.to_string()];
        terms.extend(tinysegmenter::tokenize(query));
        let vec = index.build_vector(&terms, None)?;
        Some((vec, query.to_string()))
    }
}