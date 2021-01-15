use tantivy::query::Query;
use tantivy::schema::Field;
use tantivy::schema::Value;
use tantivy::tokenizer::{TextAnalyzer, Token};
use tantivy::Searcher;
use tantivy::{Document, Score};
use htmlescape::encode_minimal;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

const DEFAULT_MAX_NUM_CHARS: usize = 150;

#[derive(Debug)]
pub struct HighlightSection {
    start: usize,
    stop: usize,
}

impl HighlightSection {
    fn new(start: usize, stop: usize) -> HighlightSection {
        HighlightSection { start, stop }
    }

    /// Returns the bounds of the `HighlightSection`.
    pub fn bounds(&self) -> (usize, usize) {
        (self.start, self.stop)
    }
}

#[derive(Debug)]
pub struct FragmentCandidate {
    score: Score,
    start_offset: usize,
    stop_offset: usize,
    num_chars: usize,
    highlighted: Vec<HighlightSection>,
}

impl FragmentCandidate {
    /// Create a basic `FragmentCandidate`
    ///
    /// `score`, `num_chars` are set to 0
    /// and `highlighted` is set to empty vec
    /// stop_offset is set to start_offset, which is taken as a param.
    fn new(start_offset: usize) -> FragmentCandidate {
        FragmentCandidate {
            score: 0.0,
            start_offset,
            stop_offset: start_offset,
            num_chars: 0,
            highlighted: vec![],
        }
    }

    /// Updates `score` and `highlighted` fields of the objects.
    ///
    /// taking the token and terms, the token is added to the fragment.
    /// if the token is one of the terms, the score
    /// and highlighted fields are updated in the fragment.
    fn try_add_token(&mut self, token: &Token, terms: &BTreeMap<String, Score>) {
        self.stop_offset = token.offset_to;

        if let Some(&score) = terms.get(&token.text.to_lowercase()) {
            self.score += score;
            self.highlighted
                .push(HighlightSection::new(token.offset_from, token.offset_to));
        }
    }
}

/// `Snippet`
/// Contains a fragment of a document, and some highlighed parts inside it.
#[derive(Debug)]
pub struct Snippet {
    fragments: String,
    highlighted: Vec<HighlightSection>,
}

const HIGHLIGHTEN_PREFIX: &str = "<b>";
const HIGHLIGHTEN_POSTFIX: &str = "</b>";

impl Snippet {
    /// Create a new, empty, `Snippet`
    pub fn empty() -> Snippet {
        Snippet {
            fragments: String::new(),
            highlighted: Vec::new(),
        }
    }

    /// Returns a hignlightned html from the `Snippet`.
    pub fn to_html(&self) -> String {
        let mut html = String::new();
        let mut start_from: usize = 0;

        for item in self.highlighted.iter() {
            html.push_str(&encode_minimal(&self.fragments[start_from..item.start]));
            html.push_str(HIGHLIGHTEN_PREFIX);
            html.push_str(&encode_minimal(&self.fragments[item.start..item.stop]));
            html.push_str(HIGHLIGHTEN_POSTFIX);
            start_from = item.stop;
        }
        html.push_str(&encode_minimal(
            &self.fragments[start_from..self.fragments.len()],
        ));
        html
    }

    pub fn to_mark(&self) -> String {
        let mut html = String::new();
        let mut start_from: usize = 0;


        for item in self.highlighted.iter() {
            html.push_str(&self.fragments[start_from..item.start]);
            html.push_str(HIGHLIGHTEN_PREFIX);
            html.push_str(&self.fragments[item.start..item.stop]);
            html.push_str(HIGHLIGHTEN_POSTFIX);
            start_from = item.stop;
        }


        html.push_str(&self.fragments[start_from..self.fragments.len()]);
        html
    }

    /// Returns a fragment from the `Snippet`.
    pub fn fragments(&self) -> &str {
        &self.fragments
    }

    /// Returns a list of higlighted positions from the `Snippet`.
    pub fn highlighted(&self) -> &[HighlightSection] {
        &self.highlighted
    }
}

/// Returns a non-empty list of "good" fragments.
///
/// If no target term is within the text, then the function
/// should return an empty Vec.
///
/// If a target term is within the text, then the returned
/// list is required to be non-empty.
///
/// The returned list is non-empty and contain less
/// than 12 possibly overlapping fragments.
///
/// All fragments should contain at least one target term
/// and have at most `max_num_chars` characters (not bytes).
///
/// It is ok to emit non-overlapping fragments, for instance,
/// one short and one long containing the same keyword, in order
/// to leave optimization opportunity to the fragment selector
/// upstream.
///
/// Fragments must be valid in the sense that `&text[fragment.start..fragment.stop]`\
/// has to be a valid string.
fn search_fragments<'a>(
    tokenizer: &TextAnalyzer,
    text: &'a str,
    terms: &BTreeMap<String, Score>,
    max_num_chars: usize,
) -> Vec<FragmentCandidate> {
    let mut token_stream = tokenizer.token_stream(text);
    let mut fragment = FragmentCandidate::new(0);
    let mut fragments: Vec<FragmentCandidate> = vec![];
    while let Some(next) = token_stream.next() {
        if (next.offset_to - fragment.start_offset) > max_num_chars {
            if fragment.score > 0.0 {
                fragments.push(fragment)
            };
            fragment = FragmentCandidate::new(next.offset_from);
        }
        fragment.try_add_token(next, &terms);
    }
    if fragment.score > 0.0 {
        fragments.push(fragment)
    }

    fragments
}

/// Returns a Snippet
///
/// Takes a vector of `FragmentCandidate`s and the text.
/// Figures out the best fragment from it and creates a snippet.
fn select_best_fragment_combination(fragments: &[FragmentCandidate], text: &str) -> Snippet {
    let best_fragment_opt = fragments.iter().max_by(|left, right| {
        let cmp_score = left
            .score
            .partial_cmp(&right.score)
            .unwrap_or(Ordering::Equal);
        if cmp_score == Ordering::Equal {
            (right.start_offset, right.stop_offset).cmp(&(left.start_offset, left.stop_offset))
        } else {
            cmp_score
        }
    });
    if let Some(fragment) = best_fragment_opt {
        let fragment_text = &text[fragment.start_offset..fragment.stop_offset];
        let highlighted = fragment
            .highlighted
            .iter()
            .map(|item| {
                HighlightSection::new(
                    item.start - fragment.start_offset,
                    item.stop - fragment.start_offset,
                )
            })
            .collect();
        Snippet {
            fragments: fragment_text.to_string(),
            highlighted,
        }
    } else {
        // when there no fragments to chose from,
        // for now create a empty snippet
        Snippet {
            fragments: String::new(),
            highlighted: vec![],
        }
    }
}

/// `SnippetGenerator`
///
/// # Example
///
/// ```rust
/// # use tantivy::query::QueryParser;
/// # use tantivy::schema::{Schema, TEXT};
/// # use tantivy::{doc, Index};
/// use tantivy::SnippetGenerator;
///
/// # fn main() -> tantivy::Result<()> {
/// #    let mut schema_builder = Schema::builder();
/// #    let text_field = schema_builder.add_text_field("text", TEXT);
/// #    let schema = schema_builder.build();
/// #    let index = Index::create_in_ram(schema);
/// #    let mut index_writer = index.writer_with_num_threads(1, 10_000_000)?;
/// #    let doc = doc!(text_field => r#"Comme je descendais des Fleuves impassibles,
/// #   Je ne me sentis plus guidé par les haleurs :
/// #  Des Peaux-Rouges criards les avaient pris pour cibles,
/// #  Les ayant cloués nus aux poteaux de couleurs.
/// #
/// #  J'étais insoucieux de tous les équipages,
/// #  Porteur de blés flamands ou de cotons anglais.
/// #  Quand avec mes haleurs ont fini ces tapages,
/// #  Les Fleuves m'ont laissé descendre où je voulais.
/// #  "#);
/// #    index_writer.add_document(doc.clone());
/// #    index_writer.commit()?;
/// #    let query_parser = QueryParser::for_index(&index, vec![text_field]);
/// // ...
/// let query = query_parser.parse_query("haleurs flamands").unwrap();
/// # let reader = index.reader()?;
/// # let searcher = reader.searcher();
/// let mut snippet_generator = SnippetGenerator::create(&searcher, &*query, text_field)?;
/// snippet_generator.set_max_num_chars(100);
/// let snippet = snippet_generator.snippet_from_doc(&doc);
/// let snippet_html: String = snippet.to_html();
/// assert_eq!(snippet_html, "Comme je descendais des Fleuves impassibles,\n  Je ne me sentis plus guidé par les <b>haleurs</b> :\n Des");
/// #    Ok(())
/// # }
/// ```
pub struct SnippetGenerator {
    terms_text: BTreeMap<String, Score>,
    tokenizer: TextAnalyzer,
    field: Field,
    max_num_chars: usize,
}

impl SnippetGenerator {
    /// Creates a new snippet generator
    pub fn create(
        searcher: &Searcher,
        query: &dyn Query,
        field: Field,
    ) -> crate::Result<SnippetGenerator> {
        let mut terms = BTreeSet::new();
        query.query_terms(&mut terms);
        let mut terms_text: BTreeMap<String, Score> = Default::default();
        for term in terms {
            if term.field() != field {
                continue;
            }
            let doc_freq = searcher.doc_freq(&term)?;
            if doc_freq > 0 {
                let score = 1.0 / (1.0 + doc_freq as Score);
                terms_text.insert(term.text().to_string(), score);
            }
        }
        let tokenizer = searcher.index().tokenizer_for_field(field)?;
        Ok(SnippetGenerator {
            terms_text,
            tokenizer,
            field,
            max_num_chars: DEFAULT_MAX_NUM_CHARS,
        })
    }

    /// Sets a maximum number of chars.
    pub fn set_max_num_chars(&mut self, max_num_chars: usize) {
        self.max_num_chars = max_num_chars;
    }

    #[cfg(test)]
    pub fn terms_text(&self) -> &BTreeMap<String, Score> {
        &self.terms_text
    }

    /// Generates a snippet for the given `Document`.
    ///
    /// This method extract the text associated to the `SnippetGenerator`'s field
    /// and computes a snippet.
    pub fn snippet_from_doc(&self, doc: &Document) -> Snippet {
        let text: String = doc
            .get_all(self.field)
            .flat_map(Value::text)
            .collect::<Vec<&str>>()
            .join(" ");
        self.snippet(&text)
    }

    /// Generates a snippet for the given text.
    pub fn snippet(&self, text: &str) -> Snippet {
        let fragment_candidates =
            search_fragments(&self.tokenizer, &text, &self.terms_text, self.max_num_chars);
        select_best_fragment_combination(&fragment_candidates[..], &text)
    }
}