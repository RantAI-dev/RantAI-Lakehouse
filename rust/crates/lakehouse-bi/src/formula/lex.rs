//! Characters to tokens. Positions are character offsets.

use super::{FormulaError, clip};

/// Longest formula, in characters.
pub const MAX_FORMULA_CHARS: usize = 2000;
/// Longest `[column name]`, in characters.
const MAX_BRACKET_CHARS: usize = 200;
/// Numbers at or above this magnitude are refused: an integer past it no
/// longer prints exactly as a double.
const MAX_NUMBER: f64 = 1e15;

/// One lexical item.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// A number, already parsed.
    Num(f64),
    /// Quoted text, unquoted.
    Str(String),
    /// A bare name (a function, a column, `and`, `true`, ...).
    Name(String),
    /// `[Column Name]`, the text between the brackets.
    Bracket(String),
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `,`
    Comma,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `=` or `==`
    Eq,
    /// `!=` or `<>`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
}

/// A token and the characters it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// What it is.
    pub tok: Tok,
    /// First character.
    pub pos: usize,
    /// Characters covered.
    pub len: usize,
}

/// Split `src` into tokens.
///
/// # Errors
///
/// A [`FormulaError`] for a formula that is too long, an unexpected
/// character, an unterminated quote or bracket, a control character inside
/// text, or a number that is malformed or too large.
pub fn tokenize(src: &str) -> Result<Vec<Token>, FormulaError> {
    let chars: Vec<char> = src.chars().collect();
    if chars.len() > MAX_FORMULA_CHARS {
        return Err(FormulaError::at(
            format!("a formula is at most {MAX_FORMULA_CHARS} characters."),
            MAX_FORMULA_CHARS,
            1,
        ));
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let (tok, len) = lex_one(&chars, i)?;
        out.push(Token { tok, pos: i, len });
        i += len;
    }
    Ok(out)
}

fn lex_one(chars: &[char], i: usize) -> Result<(Tok, usize), FormulaError> {
    let c = chars[i];
    let next = chars.get(i + 1).copied();
    let two = |tok: Tok| Ok((tok, 2));
    match c {
        '(' => Ok((Tok::LParen, 1)),
        ')' => Ok((Tok::RParen, 1)),
        ',' => Ok((Tok::Comma, 1)),
        '+' => Ok((Tok::Plus, 1)),
        '-' => Ok((Tok::Minus, 1)),
        '*' => Ok((Tok::Star, 1)),
        '/' => Ok((Tok::Slash, 1)),
        '=' if next == Some('=') => two(Tok::Eq),
        '=' => Ok((Tok::Eq, 1)),
        '!' if next == Some('=') => two(Tok::Ne),
        '<' if next == Some('>') => two(Tok::Ne),
        '<' if next == Some('=') => two(Tok::Le),
        '<' => Ok((Tok::Lt, 1)),
        '>' if next == Some('=') => two(Tok::Ge),
        '>' => Ok((Tok::Gt, 1)),
        '\'' | '"' => lex_text(chars, i, c),
        '[' => lex_bracket(chars, i),
        c if c.is_ascii_digit() => lex_number(chars, i),
        c if c.is_ascii_alphabetic() || c == '_' => {
            let len = chars[i..]
                .iter()
                .take_while(|c| c.is_ascii_alphanumeric() || **c == '_')
                .count();
            Ok((Tok::Name(chars[i..i + len].iter().collect()), len))
        }
        other => Err(FormulaError::at(
            format!("unexpected character '{}'.", clip(&other.to_string())),
            i,
            1,
        )),
    }
}

/// `'text'` or `"text"`; a quote inside is written twice. A backslash is an
/// ordinary character. Control characters are refused so that nothing
/// invisible rides into a statement.
fn lex_text(chars: &[char], start: usize, quote: char) -> Result<(Tok, usize), FormulaError> {
    let mut text = String::new();
    let mut i = start + 1;
    while i < chars.len() {
        let c = chars[i];
        if c == quote {
            if chars.get(i + 1) == Some(&quote) {
                text.push(quote);
                i += 2;
                continue;
            }
            return Ok((Tok::Str(text), i + 1 - start));
        }
        if c.is_control() && c != '\t' && c != '\n' {
            return Err(FormulaError::at(
                "text cannot contain control characters.",
                i,
                1,
            ));
        }
        text.push(c);
        i += 1;
    }
    Err(FormulaError::at(
        "this text is missing its closing quote.",
        start,
        chars.len() - start,
    ))
}

/// `[Column Name]`: everything up to the next `]`.
fn lex_bracket(chars: &[char], start: usize) -> Result<(Tok, usize), FormulaError> {
    let Some(rel) = chars[start + 1..].iter().position(|c| *c == ']') else {
        return Err(FormulaError::at(
            "this column name is missing its closing ].",
            start,
            chars.len() - start,
        ));
    };
    let name: String = chars[start + 1..start + 1 + rel].iter().collect();
    let len = rel + 2;
    if name.trim().is_empty() {
        return Err(FormulaError::at("a column name is empty.", start, len));
    }
    if rel > MAX_BRACKET_CHARS {
        return Err(FormulaError::at(
            "this column name is too long.",
            start,
            len,
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(FormulaError::at(
            "a column name cannot contain control characters.",
            start,
            len,
        ));
    }
    Ok((Tok::Bracket(name), len))
}

/// `123`, `1.5`, `2e3`, `1.5E-7`. No sign (a minus is an operator) and no
/// bare `.5`.
fn lex_number(chars: &[char], start: usize) -> Result<(Tok, usize), FormulaError> {
    let digits = |from: usize| {
        chars[from..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count()
    };
    let mut end = start + digits(start);
    if chars.get(end) == Some(&'.') && digits(end + 1) > 0 {
        end += 1 + digits(end + 1);
    }
    if matches!(chars.get(end), Some('e' | 'E')) {
        let sign = usize::from(matches!(chars.get(end + 1), Some('+' | '-')));
        let exp = digits(end + 1 + sign);
        if exp > 0 {
            end += 1 + sign + exp;
        }
    }
    let len = end - start;
    if chars
        .get(end)
        .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_' || *c == '.')
    {
        return Err(FormulaError::at(
            "this number is followed by letters; write a space or an operator.",
            start,
            len + 1,
        ));
    }
    let text: String = chars[start..end].iter().collect();
    let value: f64 = text
        .parse()
        .map_err(|_| FormulaError::at("this number is not valid.", start, len))?;
    if !value.is_finite() || value.abs() >= MAX_NUMBER {
        return Err(FormulaError::at(
            "this number is too large (below 1,000,000,000,000,000).",
            start,
            len,
        ));
    }
    Ok((Tok::Num(value), len))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        tokenize(src).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn a_formula_of_every_token_kind_is_split_with_positions() {
        let t = tokenize("[Net Sales] >= 1.5e2 and Left(name, 2) <> 'a''b'").unwrap();
        assert_eq!(t[0].tok, Tok::Bracket("Net Sales".to_owned()));
        assert_eq!((t[0].pos, t[0].len), (0, 11));
        assert_eq!(t[1].tok, Tok::Ge);
        assert_eq!(t[2].tok, Tok::Num(150.0));
        assert_eq!(t[3].tok, Tok::Name("and".to_owned()));
        assert!(t.iter().any(|t| t.tok == Tok::Ne));
        assert_eq!(t.last().unwrap().tok, Tok::Str("a'b".to_owned()));
    }

    #[test]
    fn both_quote_styles_and_a_literal_backslash_are_text() {
        assert_eq!(
            toks(r#""say ""hi""""#),
            vec![Tok::Str("say \"hi\"".to_owned())]
        );
        assert_eq!(toks(r"'a\b'"), vec![Tok::Str(r"a\b".to_owned())]);
    }

    #[test]
    fn two_character_operators_are_one_token() {
        assert_eq!(
            toks("== != <> <= >= = < >"),
            vec![
                Tok::Eq,
                Tok::Ne,
                Tok::Ne,
                Tok::Le,
                Tok::Ge,
                Tok::Eq,
                Tok::Lt,
                Tok::Gt
            ]
        );
    }

    #[test]
    fn an_unterminated_quote_or_bracket_points_at_where_it_opened() {
        let e = tokenize("1 + 'abc").unwrap_err();
        assert_eq!(e.position, 4);
        let e = tokenize("[abc + 1").unwrap_err();
        assert_eq!(e.position, 0);
    }

    #[test]
    fn an_unexpected_character_is_positioned_in_characters_not_bytes() {
        let e = tokenize("'\u{e9}\u{e9}' + #").unwrap_err();
        assert_eq!(e.position, 7, "{e:?}");
        let e = tokenize("1 ; 2").unwrap_err();
        assert_eq!(e.position, 2);
        assert!(tokenize("1 `x`").is_err());
        assert!(
            tokenize("1 -- c").is_ok_and(|t| t.len() == 4),
            "-- is two minus signs"
        );
    }

    #[test]
    fn a_number_glued_to_letters_or_too_large_is_refused() {
        assert_eq!(tokenize("12abc").unwrap_err().position, 0);
        assert_eq!(tokenize("1 + 1e400").unwrap_err().position, 4);
        assert!(tokenize("1e15").is_err());
        assert!(tokenize("999999999999999").is_ok());
        assert!(tokenize(".5").is_err());
    }

    #[test]
    fn control_characters_in_text_and_names_are_refused() {
        assert!(tokenize("'a\u{0}b'").is_err());
        assert!(tokenize("[a\u{7}b]").is_err());
        assert!(tokenize("'a\tb'").is_ok());
        assert!(tokenize("[]").is_err());
        assert!(tokenize("[   ]").is_err());
    }

    #[test]
    fn the_length_limit_counts_characters() {
        let ok: String = "1+".repeat(999) + "1";
        assert_eq!(ok.chars().count(), 1999);
        assert!(tokenize(&ok).is_ok());
        let long = "a".repeat(2001);
        assert_eq!(tokenize(&long).unwrap_err().position, 2000);
        let wide = "\u{e9}".repeat(2000);
        assert!(tokenize(&format!("'{wide}'")).is_err(), "2002 characters");
    }
}
