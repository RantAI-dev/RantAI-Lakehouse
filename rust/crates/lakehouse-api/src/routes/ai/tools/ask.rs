//! `ask_user`: the model asks which reading of an unclear word the person
//! means, and offers the readings as options.
//!
//! The tool stores nothing and runs no query. It only checks the question
//! and echoes it back, so the console can show the options as buttons. The
//! console's click is what saves the person's answer (`PUT /api/ai/terms`);
//! the model has no way to write a term itself.

use serde_json::{Map, Value, json};

use super::arg_str;

/// Longest `term`, in characters. Matches the store's rule for a saved term
/// (`lakehouse_store::chat_term::MAX_TERM_CHARS`), so an option the console
/// saves cannot be refused for its term.
const MAX_TERM_CHARS: usize = 60;
/// Longest `question`, in characters: one sentence.
const MAX_QUESTION_CHARS: usize = 200;
/// Fewest and most `options`. One option is not a choice, and more than four
/// is a list a small model cannot choose between.
const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 4;
/// Longest single option, in characters: it is a button label.
const MAX_OPTION_CHARS: usize = 80;

/// Validates the arguments and returns `{asked: true, term, question,
/// options}`, or `{error}` naming the first broken rule. Lengths count
/// characters, not bytes, and values are trimmed before they are counted.
pub(super) fn ask_user(args: &Map<String, Value>) -> Value {
    let term = arg_str(args, "term").trim().to_owned();
    if !(1..=MAX_TERM_CHARS).contains(&term.chars().count()) {
        return json!({ "error": format!("term must be 1 to {MAX_TERM_CHARS} characters") });
    }
    let question = arg_str(args, "question").trim().to_owned();
    if !(1..=MAX_QUESTION_CHARS).contains(&question.chars().count()) {
        return json!({ "error": format!("question must be 1 to {MAX_QUESTION_CHARS} characters") });
    }
    let Some(raw) = args.get("options").and_then(Value::as_array) else {
        return json!({ "error": format!("options must be a list of {MIN_OPTIONS} to {MAX_OPTIONS} strings") });
    };
    if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&raw.len()) {
        return json!({ "error": format!("options must hold {MIN_OPTIONS} to {MAX_OPTIONS} items") });
    }
    let mut options: Vec<String> = Vec::with_capacity(raw.len());
    for item in raw {
        let Some(text) = item.as_str() else {
            return json!({ "error": "every option must be a string" });
        };
        let text = text.trim();
        if !(1..=MAX_OPTION_CHARS).contains(&text.chars().count()) {
            return json!({ "error": format!("each option must be 1 to {MAX_OPTION_CHARS} characters") });
        }
        if options
            .iter()
            .any(|seen| seen.to_lowercase() == text.to_lowercase())
        {
            return json!({ "error": "options must differ from each other" });
        }
        options.push(text.to_owned());
    }
    json!({ "asked": true, "term": term, "question": question, "options": options })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn args(term: &str, question: &str, options: Value) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("term".to_owned(), json!(term));
        map.insert("question".to_owned(), json!(question));
        map.insert("options".to_owned(), options);
        map
    }

    fn error_of(result: &Value) -> &str {
        result["error"]
            .as_str()
            .unwrap_or_else(|| panic!("expected an error, got {result}"))
    }

    #[test]
    fn a_valid_question_is_echoed_back_trimmed() {
        let result = ask_user(&args(
            " hotel ",
            " Which one do you mean? ",
            json!([" hotel_a ", "hotel_b"]),
        ));
        assert_eq!(
            result,
            json!({
                "asked": true,
                "term": "hotel",
                "question": "Which one do you mean?",
                "options": ["hotel_a", "hotel_b"],
            })
        );
    }

    #[test]
    fn the_term_must_be_one_to_sixty_characters() {
        let options = json!(["a", "b"]);
        assert!(error_of(&ask_user(&args("", "q?", options.clone()))).starts_with("term "));
        assert!(error_of(&ask_user(&args("   ", "q?", options.clone()))).starts_with("term "));
        assert!(error_of(&ask_user(&args(&"x".repeat(61), "q?", options.clone()))).contains("60"));
        assert_eq!(
            ask_user(&args(&"x".repeat(60), "q?", options))["asked"],
            json!(true)
        );
    }

    #[test]
    fn the_question_must_be_one_to_two_hundred_characters() {
        let options = json!(["a", "b"]);
        assert!(error_of(&ask_user(&args("t", "", options.clone()))).starts_with("question "));
        assert!(error_of(&ask_user(&args("t", &"q".repeat(201), options.clone()))).contains("200"));
        assert_eq!(
            ask_user(&args("t", &"q".repeat(200), options))["asked"],
            json!(true)
        );
    }

    #[test]
    fn there_must_be_two_to_four_options() {
        assert!(error_of(&ask_user(&args("t", "q?", json!(["a"])))).contains("2 to 4"));
        assert!(
            error_of(&ask_user(&args(
                "t",
                "q?",
                json!(["a", "b", "c", "d", "e"])
            )))
            .contains("2 to 4")
        );
        assert_eq!(
            ask_user(&args("t", "q?", json!(["a", "b", "c", "d"])))["asked"],
            json!(true)
        );
        assert!(error_of(&ask_user(&args("t", "q?", json!(null)))).starts_with("options "));
    }

    #[test]
    fn each_option_must_be_one_to_eighty_characters_and_a_string() {
        assert!(
            error_of(&ask_user(&args("t", "q?", json!(["a", "  "])))).starts_with("each option")
        );
        assert!(error_of(&ask_user(&args("t", "q?", json!(["a", "b".repeat(81)])))).contains("80"));
        assert_eq!(
            ask_user(&args("t", "q?", json!(["a", "b".repeat(80)])))["asked"],
            json!(true)
        );
        assert!(error_of(&ask_user(&args("t", "q?", json!(["a", 7])))).starts_with("every option"));
    }

    #[test]
    fn two_options_that_differ_only_in_case_or_padding_are_refused() {
        assert!(
            error_of(&ask_user(&args("t", "q?", json!(["Hotel", " hotel "])))).contains("differ")
        );
    }

    #[test]
    fn lengths_count_characters_not_bytes() {
        // 60 two-byte characters are 120 bytes; 61 of them are one over.
        let sixty = "é".repeat(60);
        let options = json!(["a", "b"]);
        assert_eq!(
            ask_user(&args(&sixty, "q?", options.clone()))["asked"],
            json!(true)
        );
        assert!(error_of(&ask_user(&args(&"é".repeat(61), "q?", options))).contains("60"));
        let eighty = "é".repeat(80);
        assert_eq!(
            ask_user(&args("t", "q?", json!(["a", eighty])))["asked"],
            json!(true)
        );
    }
}
