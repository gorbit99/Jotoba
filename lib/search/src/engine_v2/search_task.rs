use super::{result::SearchResult, result_item::ResultItem, SearchEngine};
use error::Error;
use itertools::Itertools;
use resources::parse::jmdict::languages::Language;
use std::{collections::BinaryHeap, marker::PhantomData};
use vector_space_model::DocumentVector;

pub struct SearchTask<'a, T>
where
    T: SearchEngine,
{
    /// Search query
    queries: Vec<(&'a str, Option<Language>)>,
    /// filter out vectors
    vec_filter: Option<Box<dyn Fn(&T::Document) -> bool>>,
    /// Filter out results
    res_filter: Option<Box<dyn Fn(&T::Output) -> bool>>,
    /// Custom result order function
    order: Option<Box<dyn Fn(&T::Output, f32, &str, Option<Language>) -> usize>>,
    /// Min relevance returned from vector space algo
    threshold: f32,
    limit: usize,
    vector_limit: usize,
    offset: usize,
    allow_align: bool,
    phantom: PhantomData<T>,
}

impl<'a, T> SearchTask<'a, T>
where
    T: SearchEngine,
{
    /// Creates a new Search task
    #[inline]
    pub fn new(query: &'a str) -> Self {
        let mut task = Self::default();
        task.queries.push((query, None));
        task
    }

    /// Creates a new Search task with a query assigned language
    pub fn with_language(query: &'a str, language: Language) -> Self {
        let mut task = Self::default();
        task.queries.push((query, Some(language)));
        task
    }

    /// Adds another query to look out for to the search task
    pub fn add_language_query(mut self, query: &'a str, language: Language) -> Self {
        self.queries.push((query, Some(language)));
        self
    }

    /// Adds another query to look out for to the search task
    pub fn add_query(&mut self, query: &'a str) {
        self.queries.push((query, None));
    }

    /// Set the total limit. This is the max amount of vectors which will be loaded and processed
    pub fn limit(mut self, total_limit: usize) -> Self {
        self.limit = total_limit;
        self
    }

    /// Sets the search task's threshold. This does not apply on the final score, which can be
    /// overwritten by `order` but applies to the vector space relevance itself.
    pub fn threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }

    /// Sets the offeset of the search. Can be used for pagination. Requires output of search being
    /// directly used and not manually reordered
    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }

    /// Set the search task's vector filter.
    pub fn set_vector_filter<F: 'static>(&mut self, vec_filter: F)
    where
        F: Fn(&T::Document) -> bool,
    {
        self.vec_filter = Some(Box::new(vec_filter));
    }

    /// Set the search task's result filter.
    pub fn set_result_filter<F: 'static>(&mut self, res_filter: F)
    where
        F: Fn(&T::Output) -> bool,
    {
        self.res_filter = Some(Box::new(res_filter));
    }

    /// Set the search task's custom order function
    pub fn set_order_fn<F: 'static>(&mut self, res_filter: F)
    where
        F: Fn(&T::Output, f32, &str, Option<Language>) -> usize,
    {
        self.order = Some(Box::new(res_filter));
    }

    /// Returns the amount of queries, this search task is going to look out for
    #[inline]
    pub fn query_count(&self) -> usize {
        self.queries.len()
    }

    /// Returns `true` if the search task's query is a term in the corresponding index
    #[inline]
    pub fn has_term(&self) -> bool {
        self.queries.iter().any(|(query, language)| {
            T::get_index(*language)
                .map(|i| i.get_indexer().clone().find_term(query).is_some())
                .unwrap_or(false)
        })
    }

    /// Runs the search task and returns the result.
    pub fn find(&self) -> Result<SearchResult<&T::Output>, Error> {
        let items = self
            .get_queries()
            .map(|(q_str, vec, lang)| self.find_by_vec(vec, q_str, lang))
            .collect::<Result<Vec<_>, Error>>()?
            .into_iter()
            .flatten()
            .unique_by(|a| a.item)
            .collect::<Vec<_>>();

        let heap: BinaryHeap<ResultItem<&T::Output>> = BinaryHeap::from(items);

        Ok(SearchResult::from_binary_heap(
            heap,
            self.offset,
            self.limit,
        ))
    }

    /// Returns an iterator over all queries in form of document vectors and its assigned language
    fn get_queries<'b>(
        &'b self,
    ) -> impl Iterator<Item = (&'b str, DocumentVector<T::GenDoc>, Option<Language>)> {
        self.queries.iter().filter_map(move |(q_str, lang)| {
            let index = T::get_index(*lang).expect("Lang not loaded");

            // align query
            let aligned_query = self
                .allow_align
                .then(|| T::align_query(q_str, index, *lang))
                .flatten()
                .unwrap_or(q_str);

            let vec = T::gen_query_vector(index, aligned_query)?;

            Some((aligned_query, vec, *lang))
        })
    }

    fn find_by_vec(
        &self,
        q_vec: DocumentVector<T::GenDoc>,
        q_str: &str,
        language: Option<Language>,
    ) -> Result<Vec<ResultItem<&T::Output>>, Error> {
        let index = T::get_index(language);
        if index.is_none() {
            log::error!("Failed to retrieve {:?} index with language", language);
            return Err(Error::Unexpected);
        }
        let index = index.unwrap();

        let mut vec_store = index.get_vector_store().clone();
        let query_dimensions: Vec<_> = q_vec.vector().vec_indices().collect();

        // Retrieve all document vectors that share at least one dimension with the query vector
        let document_vectors = vec_store
            .get_all_iter(&query_dimensions)
            .take(self.vector_limit);

        self.result_from_doc_vectors(document_vectors, &q_vec, q_str, language)
    }

    fn result_from_doc_vectors(
        &self,
        document_vectors: impl Iterator<Item = DocumentVector<T::Document>>,
        q_vec: &DocumentVector<T::GenDoc>,
        q_str: &str,
        language: Option<Language>,
    ) -> Result<Vec<ResultItem<&T::Output>>, Error> {
        let storage = resources::get();

        let res = document_vectors
            .filter_map(|i| {
                if !self.filter_vector(&i.document) {
                    return None;
                }

                let similarity = i.similarity(&q_vec);
                if similarity <= self.threshold {
                    return None;
                }

                // Retrieve `Output` values for given documents
                let res = T::doc_to_output(storage, &i.document)?
                    .into_iter()
                    .map(move |i| (similarity, i));

                Some(res)
            })
            .flatten()
            .filter(|i| self.filter_result(&i.1))
            .map(|(rel, item)| {
                let relevance = self.calculate_score(item, rel, q_str, language);

                language
                    .map(|i| ResultItem::with_language(item, relevance, i))
                    .unwrap_or(ResultItem::new(item, relevance))
            })
            .collect::<Vec<_>>();
        Ok(res)
    }

    /// Calculates the score using a custom function if provided or just `rel` otherwise
    #[inline]
    fn calculate_score(
        &self,
        item: &T::Output,
        rel: f32,
        query: &str,
        language: Option<Language>,
    ) -> usize {
        self.order
            .as_ref()
            .map(|i| i(item, rel, query, language))
            .unwrap_or((rel * 100f32) as usize)
    }

    #[inline]
    fn filter_result(&self, output: &T::Output) -> bool {
        self.res_filter.as_ref().map(|i| i(output)).unwrap_or(true)
    }

    #[inline]
    fn filter_vector(&self, vec: &T::Document) -> bool {
        self.vec_filter.as_ref().map(|i| i(vec)).unwrap_or(true)
    }
}

impl<'a, T: SearchEngine> Default for SearchTask<'a, T> {
    #[inline]
    fn default() -> Self {
        Self {
            queries: Vec::with_capacity(1),
            vec_filter: None,
            res_filter: None,
            order: None,
            threshold: 0.2,
            limit: 1000,
            vector_limit: 100_000,
            offset: 0,
            allow_align: true,
            phantom: PhantomData::default(),
        }
    }
}
