use crate::{
    engine::{words::native, SearchTask},
    query::Query,
    word::{filter::WordFilter, order},
};

/// Helper for creating SearchTask for foreign queries
pub struct NativeSearch<'a> {
    query: &'a Query,
    query_str: &'a str,
}

impl<'a> NativeSearch<'a> {
    pub(crate) fn new(query: &'a Query, query_str: &'a str) -> Self {
        Self { query, query_str }
    }

    pub fn task(&self) -> SearchTask<native::Engine> {
        let mut task: SearchTask<native::Engine> = SearchTask::new(self.query_str);

        let original_query = self.query.raw_query.clone();
        task.with_custom_order(move |item| {
            order::japanese_search_order(item, Some(&original_query))
        });

        let filter = WordFilter::new(self.query.clone());
        task.set_result_filter(move |item| !filter.filter_word(*item));

        task
    }

    /// Returns `true` if Native search has `term` in index
    #[inline]
    pub fn has_term(term: &str) -> bool {
        SearchTask::<native::Engine>::new(term).has_term()
    }
}
