//! Persistent core tokenization plus SQLite FTS5 compatibility sanitization.
//!
//! Read this before assuming the core supports Boolean search. It does not.
//!
//! [`format_sqlite_fts5_query`] emits SQLite FTS5 `MATCH` syntax and is exercised against a real
//! FTS5 virtual table in this module's tests. **No core path calls it.** The core search
//! projection `documents_fts` is a plain DuckDB table. The core's persistent token projection
//! implements the same intentionally small contract as the typed frontend: every unqualified
//! text token must occur, each quoted phrase ([`core_query_phrases`]) must occur as consecutive
//! tokens, while agency, portal, status, and flag qualifiers travel as separate
//! typed command parameters. Boolean operators and field clauses are deliberately not part of
//! that contract.
//!
//! The sanitizer is retained rather than deleted because it is the piece a real Boolean search
//! needs and because the SQLite attachment boundary can still present FTS5 tables.
//!
//! The DuckDB core persists [`token_frequencies`] and searches with [`core_query_tokens`].

pub(crate) use crate::normalization::normalize_index_text as clean_plain_text_for_index;
use std::collections::{BTreeMap, BTreeSet};

const MAXIMUM_QUERY_BYTES: usize = 16 * 1024;
const MAXIMUM_QUERY_LEXEMES: usize = 512;
const MAXIMUM_CLAUSE_BYTES: usize = 512;
const MAXIMUM_GROUP_DEPTH: usize = 32;

const MAXIMUM_INDEX_TOKEN_CHARACTERS: usize = 128;

pub const MAXIMUM_CORE_QUERY_BYTES: usize = MAXIMUM_QUERY_BYTES;
pub const MAXIMUM_CORE_QUERY_TOKENS: usize = MAXIMUM_QUERY_LEXEMES;
const MAXIMUM_CORE_QUERY_PHRASE_TOKENS: usize = 64;

pub fn token_frequencies(text: &str) -> BTreeMap<String, u64> {
    let mut frequencies = BTreeMap::new();
    for token in text
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| {
            token
                .to_lowercase()
                .chars()
                .take(MAXIMUM_INDEX_TOKEN_CHARACTERS)
                .collect::<String>()
        })
    {
        let frequency = frequencies.entry(token).or_insert(0_u64);
        *frequency = frequency.saturating_add(1);
    }
    frequencies
}

/// Extracts the distinct terms the typed frontend sends as full-text content. Structured fields
/// are transported separately. All returned terms are required by the core query.
pub fn core_query_tokens(query: &str) -> Result<Vec<String>, &'static str> {
    if query.len() > MAXIMUM_CORE_QUERY_BYTES {
        return Err("Search text exceeds the 16 KiB query limit");
    }
    let tokens = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| {
            token
                .to_lowercase()
                .chars()
                .take(MAXIMUM_INDEX_TOKEN_CHARACTERS)
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    if tokens.len() > MAXIMUM_CORE_QUERY_TOKENS {
        return Err("Search text exceeds the 512-token query limit");
    }
    Ok(tokens
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

/// The quoted phrases of a query, each as its lowercased tokens in order. A phrase must occur as
/// consecutive tokens, so `"body camera"` does not match a record that only says "camera ... body".
/// A one-token phrase adds nothing beyond [`core_query_tokens`] and is omitted, as is an unclosed
/// quote. Tokens are not truncated: the phrase is matched against the full text, not the index.
pub fn core_query_phrases(query: &str) -> Result<Vec<Vec<String>>, &'static str> {
    let segments: Vec<&str> = query.split('"').collect();
    // Odd segments sit inside quotes; the last one is inside only if a closing quote follows it.
    let closed = &segments[..segments.len() - 1];
    let mut phrases: Vec<Vec<String>> = closed
        .iter()
        .skip(1)
        .step_by(2)
        .map(|quoted| {
            quoted
                .split(|character: char| !character.is_alphanumeric())
                .filter(|token| !token.is_empty())
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .filter(|tokens| tokens.len() > 1)
        .collect();
    // Each phrase compiles to one regular expression; DuckDB's engine refused one of 512 words as
    // too large, so the limit is stated here instead of surfacing as an engine error.
    if phrases
        .iter()
        .any(|tokens| tokens.len() > MAXIMUM_CORE_QUERY_PHRASE_TOKENS)
    {
        return Err("A quoted phrase can hold at most 64 words");
    }
    phrases.sort();
    phrases.dedup();
    Ok(phrases)
}

/// Converts untrusted user input into a valid, parameter-ready SQLite FTS5 Boolean expression.
///
/// Not used by the DuckDB core search path; see the module documentation.
///
/// Only alphanumeric terms, quoted phrases, the `AND`/`OR`/`NOT` operators, and balanced
/// grouping parentheses survive normalization. Adjacent terms and groups receive an implicit
/// `AND`. Operators are parsed case-insensitively outside quoted phrases, with FTS5-compatible
/// precedence (`NOT`, then `AND`, then `OR`). Malformed operators and parentheses are recovered
/// into the largest valid expression possible; input that contains no searchable term becomes an
/// empty string.
pub fn format_sqlite_fts5_query(user_query: &str) -> String {
    let lexemes = lex_query(user_query);
    QueryParser::new(lexemes)
        .parse()
        .map(|expression| expression.render(0))
        .unwrap_or_default()
}

/// Former name of [`format_sqlite_fts5_query`], retained so existing callers keep compiling.
///
/// The name was qualified because an unqualified `fts5` in a DuckDB-backed crate reads as a
/// description of the core, which has no FTS5 anything.
pub use self::format_sqlite_fts5_query as format_fts5_query;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BooleanOperator {
    And,
    Or,
    Not,
}

impl BooleanOperator {
    fn as_str(self) -> &'static str {
        match self {
            Self::And => "AND",
            Self::Or => "OR",
            Self::Not => "NOT",
        }
    }

    fn precedence(self) -> u8 {
        match self {
            Self::Or => 1,
            Self::And => 2,
            Self::Not => 3,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum QueryLexeme {
    Clause(String),
    Operator(BooleanOperator),
    LeftParenthesis,
    RightParenthesis,
}

#[derive(Debug, Eq, PartialEq)]
enum QueryExpression {
    Clause(String),
    Binary {
        left: Box<QueryExpression>,
        operator: BooleanOperator,
        right: Box<QueryExpression>,
    },
}

impl QueryExpression {
    fn binary(left: QueryExpression, operator: BooleanOperator, right: QueryExpression) -> Self {
        Self::Binary {
            left: Box::new(left),
            operator,
            right: Box::new(right),
        }
    }

    fn render(&self, minimum_precedence: u8) -> String {
        match self {
            Self::Clause(clause) => format!("\"{clause}\""),
            Self::Binary {
                left,
                operator,
                right,
            } => {
                let precedence = operator.precedence();
                let left = left.render(precedence);
                // Requiring greater precedence on the right preserves an explicitly grouped,
                // right-nested expression when the child uses the same operator precedence.
                let right = right.render(precedence + 1);
                let rendered = format!("{left} {} {right}", operator.as_str());
                if precedence < minimum_precedence {
                    format!("({rendered})")
                } else {
                    rendered
                }
            }
        }
    }
}

struct QueryParser {
    lexemes: Vec<QueryLexeme>,
    position: usize,
}

impl QueryParser {
    fn new(lexemes: Vec<QueryLexeme>) -> Self {
        Self {
            lexemes,
            position: 0,
        }
    }

    fn parse(mut self) -> Option<QueryExpression> {
        self.parse_or_expression()
    }

    fn parse_or_expression(&mut self) -> Option<QueryExpression> {
        let mut expression = self.parse_and_expression()?;

        while self.consume_operator(BooleanOperator::Or) {
            if let Some(right) = self.parse_and_expression() {
                expression = QueryExpression::binary(expression, BooleanOperator::Or, right);
            }
        }

        Some(expression)
    }

    fn parse_and_expression(&mut self) -> Option<QueryExpression> {
        let mut expression = self.parse_not_expression()?;

        loop {
            if self.consume_operator(BooleanOperator::And) {
                // Users commonly write `A AND NOT B`, while FTS5 expresses exclusion with the
                // binary form `A NOT B`. Normalize that spelling without permitting unary SQL/FTS
                // syntax to reach SQLite.
                if self.consume_operator(BooleanOperator::Not) {
                    if let Some(right) = self.parse_not_expression() {
                        expression =
                            QueryExpression::binary(expression, BooleanOperator::Not, right);
                    }
                } else if let Some(right) = self.parse_not_expression() {
                    expression = QueryExpression::binary(expression, BooleanOperator::And, right);
                }
                continue;
            }

            if self.starts_primary_expression() {
                if let Some(right) = self.parse_not_expression() {
                    expression = QueryExpression::binary(expression, BooleanOperator::And, right);
                    continue;
                }
            }

            break;
        }

        Some(expression)
    }

    fn parse_not_expression(&mut self) -> Option<QueryExpression> {
        let mut expression = self.parse_primary_expression()?;

        while self.consume_operator(BooleanOperator::Not) {
            if let Some(right) = self.parse_primary_expression() {
                expression = QueryExpression::binary(expression, BooleanOperator::Not, right);
            }
        }

        Some(expression)
    }

    fn parse_primary_expression(&mut self) -> Option<QueryExpression> {
        loop {
            match self.lexemes.get(self.position) {
                Some(QueryLexeme::Clause(clause)) => {
                    let clause = clause.clone();
                    self.position += 1;
                    return Some(QueryExpression::Clause(clause));
                }
                Some(QueryLexeme::LeftParenthesis) => {
                    self.position += 1;
                    let nested = self.parse_or_expression();
                    if matches!(
                        self.lexemes.get(self.position),
                        Some(QueryLexeme::RightParenthesis)
                    ) {
                        self.position += 1;
                    }
                    if nested.is_some() {
                        return nested;
                    }
                }
                Some(QueryLexeme::Operator(_)) => {
                    // A leading, repeated, or otherwise orphaned operator has no safe FTS5
                    // meaning. Drop it and continue recovering the next searchable expression.
                    self.position += 1;
                }
                Some(QueryLexeme::RightParenthesis) | None => return None,
            }
        }
    }

    fn starts_primary_expression(&self) -> bool {
        matches!(
            self.lexemes.get(self.position),
            Some(QueryLexeme::Clause(_) | QueryLexeme::LeftParenthesis)
        )
    }

    fn consume_operator(&mut self, expected: BooleanOperator) -> bool {
        if matches!(
            self.lexemes.get(self.position),
            Some(QueryLexeme::Operator(operator)) if *operator == expected
        ) {
            self.position += 1;
            true
        } else {
            false
        }
    }
}

fn lex_query(user_query: &str) -> Vec<QueryLexeme> {
    let mut lexemes = Vec::new();
    let mut buffer = String::new();
    let mut inside_phrase = false;
    let mut open_parentheses = 0_usize;
    let mut consumed_bytes = 0_usize;

    for character in user_query.chars() {
        if consumed_bytes
            .checked_add(character.len_utf8())
            .is_none_or(|size| size > MAXIMUM_QUERY_BYTES)
            || lexemes.len() >= MAXIMUM_QUERY_LEXEMES
        {
            break;
        }
        consumed_bytes += character.len_utf8();
        if inside_phrase {
            match character {
                '"' => {
                    push_phrase_clause(&mut lexemes, &mut buffer);
                    inside_phrase = false;
                }
                character if character.is_alphanumeric() => {
                    push_clause_character(&mut buffer, character)
                }
                _ => push_phrase_separator(&mut buffer),
            }
            continue;
        }

        match character {
            '"' => {
                push_unquoted_lexeme(&mut lexemes, &mut buffer);
                inside_phrase = true;
            }
            '(' => {
                push_unquoted_lexeme(&mut lexemes, &mut buffer);
                if open_parentheses < MAXIMUM_GROUP_DEPTH && lexemes.len() < MAXIMUM_QUERY_LEXEMES {
                    lexemes.push(QueryLexeme::LeftParenthesis);
                    open_parentheses += 1;
                }
            }
            ')' => {
                push_unquoted_lexeme(&mut lexemes, &mut buffer);
                if open_parentheses > 0 && lexemes.len() < MAXIMUM_QUERY_LEXEMES {
                    lexemes.push(QueryLexeme::RightParenthesis);
                    open_parentheses -= 1;
                }
            }
            character if character.is_alphanumeric() => {
                push_clause_character(&mut buffer, character)
            }
            _ => push_unquoted_lexeme(&mut lexemes, &mut buffer),
        }
    }

    if inside_phrase {
        push_phrase_clause(&mut lexemes, &mut buffer);
    } else {
        push_unquoted_lexeme(&mut lexemes, &mut buffer);
    }

    lexemes
}

fn push_phrase_separator(buffer: &mut String) {
    if !buffer.is_empty() && !buffer.ends_with(' ') && buffer.len() < MAXIMUM_CLAUSE_BYTES {
        buffer.push(' ');
    }
}

fn push_clause_character(buffer: &mut String, character: char) {
    if buffer
        .len()
        .checked_add(character.len_utf8())
        .is_some_and(|size| size <= MAXIMUM_CLAUSE_BYTES)
    {
        buffer.push(character);
    }
}

fn push_phrase_clause(lexemes: &mut Vec<QueryLexeme>, buffer: &mut String) {
    let phrase = buffer.trim();
    if !phrase.is_empty() && lexemes.len() < MAXIMUM_QUERY_LEXEMES {
        lexemes.push(QueryLexeme::Clause(phrase.to_string()));
    }
    buffer.clear();
}

fn push_unquoted_lexeme(lexemes: &mut Vec<QueryLexeme>, buffer: &mut String) {
    if buffer.is_empty() || lexemes.len() >= MAXIMUM_QUERY_LEXEMES {
        return;
    }

    let lexeme = if buffer.eq_ignore_ascii_case("AND") {
        QueryLexeme::Operator(BooleanOperator::And)
    } else if buffer.eq_ignore_ascii_case("OR") {
        QueryLexeme::Operator(BooleanOperator::Or)
    } else if buffer.eq_ignore_ascii_case("NOT") {
        QueryLexeme::Operator(BooleanOperator::Not)
    } else {
        QueryLexeme::Clause(std::mem::take(buffer))
    };
    lexemes.push(lexeme);
    buffer.clear();
}

#[cfg(test)]
mod tests {
    use rusqlite::{params, Connection};

    use super::{clean_plain_text_for_index, format_fts5_query, format_sqlite_fts5_query};

    /// The compatibility alias must keep resolving to the renamed function.
    #[test]
    fn the_previous_sanitizer_name_still_resolves() {
        let renamed: fn(&str) -> String = format_sqlite_fts5_query;
        let alias: fn(&str) -> String = format_fts5_query;
        for query in ["housing notice", "(a OR b) AND NOT c", ") OR OR ("] {
            assert_eq!(alias(query), renamed(query));
        }
    }

    #[test]
    fn normalizes_index_text_and_basic_queries() {
        assert_eq!(
            clean_plain_text_for_index("  First\r\nsecond\0\tthird.  "),
            "First second third."
        );
        assert_eq!(
            format_fts5_query("housing notice"),
            "\"housing\" AND \"notice\""
        );
        assert_eq!(
            format_fts5_query("\"public records request\""),
            "\"public records request\""
        );
    }

    #[test]
    fn preserves_boolean_semantics_and_sanitizes_malformed_input() {
        assert_eq!(
            format_fts5_query("(housing OR eviction) notice"),
            "(\"housing\" OR \"eviction\") AND \"notice\""
        );
        assert_eq!(
            format_fts5_query("records AND NOT (draft OR privileged)"),
            "\"records\" NOT (\"draft\" OR \"privileged\")"
        );
        assert_eq!(
            format_fts5_query("title:secret* OR {records}"),
            "\"title\" AND \"secret\" OR \"records\""
        );
        assert_eq!(format_fts5_query(") OR OR (records AND"), "\"records\"");
        let formatted = format_fts5_query("(housing OR eviction) AND NOT draft");
        assert_eq!(format_fts5_query(&formatted), formatted);
    }

    #[test]
    fn generated_boolean_queries_execute_against_sqlite_fts5() {
        let connection = Connection::open_in_memory().expect("SQLite must open");
        connection
            .execute_batch(
                "CREATE VIRTUAL TABLE documents_fts USING fts5(extracted_text);
                 INSERT INTO documents_fts(extracted_text) VALUES
                    ('housing eviction notice'),
                    ('housing draft'),
                    ('public records appeal');",
            )
            .expect("FTS fixture must initialize");

        for (user_query, expected_row_ids) in [
            ("(housing OR eviction) notice", vec![1_i64]),
            ("housing AND NOT draft", vec![1_i64]),
            ("\"public records\" OR appeal", vec![3_i64]),
            (") OR OR (records AND", vec![3_i64]),
        ] {
            let formatted = format_fts5_query(user_query);
            let actual_row_ids = connection
                .prepare(
                    "SELECT rowid
                     FROM documents_fts
                     WHERE documents_fts MATCH ?1
                     ORDER BY rowid",
                )
                .expect("FTS query must prepare")
                .query_map(params![formatted], |row| row.get::<_, i64>(0))
                .expect("every generated expression must be accepted by FTS5")
                .collect::<rusqlite::Result<Vec<_>>>()
                .expect("FTS result identifiers must decode");
            assert_eq!(actual_row_ids, expected_row_ids);
        }
    }
}
