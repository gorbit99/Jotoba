use crate::{
    engine::{search_task::cpushable::FilteredMaxCounter, sentences::foreign, SearchTask},
    executor::{out_builder::OutputBuilder, producer::Producer, searchable::Searchable},
    query::{Query, QueryLang},
    sentence::Search,
};
use types::jotoba::languages::Language;

/// Producer for sentences by foreign keywords
pub struct ForeignProducer<'a> {
    query: &'a Query,
    language: Language,
}

impl<'a> ForeignProducer<'a> {
    pub fn new(query: &'a Query, language: Language) -> Self {
        Self { query, language }
    }

    fn task(&self) -> SearchTask<foreign::Engine> {
        let query_str = &self.query.query_str;

        let mut search_task: SearchTask<foreign::Engine> =
            SearchTask::with_language(query_str, self.language);

        let query_c = self.query.clone();
        search_task
            .set_result_filter(move |sentence| super::filter::filter_sentence(&query_c, sentence));

        let query_c = self.query.clone();
        search_task.with_custom_order(move |item| {
            let mut rel = (item.vec_simiarity() * 100000f32) as usize;

            if item.item().has_translation(query_c.settings.user_lang) {
                rel += 550;
            }

            rel
        });

        search_task
    }
}

impl<'a> Producer for ForeignProducer<'a> {
    type Target = Search<'a>;

    fn produce(
        &self,
        out: &mut OutputBuilder<
            <Self::Target as Searchable>::Item,
            <Self::Target as Searchable>::ResAdd,
        >,
    ) {
        self.task().find_to(out);
    }

    fn should_run(&self, _already_found: usize) -> bool {
        self.query.form.is_normal() && self.query.q_lang == QueryLang::Foreign
    }

    fn estimate_to(&self, out: &mut FilteredMaxCounter<<Self::Target as Searchable>::Item>) {
        self.task().estimate_to(out);
    }
}
