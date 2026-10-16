//! Tokens to a syntax tree.
//!
//! ```text
//! expr    = or
//! or      = and ( "or" and )*
//! and     = not ( "and" not )*
//! not     = "not" not | compare
//! compare = sum ( ("=" | "!=" | "<" | "<=" | ">" | ">=") sum )?
//! sum     = product ( ("+" | "-") product )*
//! product = unary ( ("*" | "/") unary )*
//! unary   = "-" unary | primary
//! primary = number | text | [column] | name | name "(" args ")" | "(" expr ")"
//! ```
//!
//! A comparison does not chain (`a < b < c` is refused with a hint to use
//! `and`), so a formula reads one way only.

use super::lex::{Tok, Token, tokenize};
use super::{FormulaError, clip};

/// Deepest nesting of parentheses, calls and prefix operators.
pub const MAX_DEPTH: usize = 32;

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `and`
    And,
    /// `or`
    Or,
}

/// What a node is.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// A number.
    Num(f64),
    /// Text.
    Str(String),
    /// `true` or `false`.
    Bool(bool),
    /// A column or another field, by name (bare or in brackets).
    Name(String),
    /// A function call; `name` as written.
    Call {
        /// The function name as the person wrote it.
        name: String,
        /// The arguments.
        args: Vec<Node>,
    },
    /// `-x`
    Neg(Box<Node>),
    /// `not x`
    Not(Box<Node>),
    /// `a op b`
    Bin(BinOp, Box<Node>, Box<Node>),
}

/// A node and the characters it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// What it is.
    pub kind: Kind,
    /// First character.
    pub pos: usize,
    /// Characters covered.
    pub len: usize,
}

/// Parse a formula.
///
/// # Errors
///
/// A [`FormulaError`] from the tokenizer, or for a syntax mistake, an empty
/// formula, or nesting deeper than [`MAX_DEPTH`].
pub fn parse(src: &str) -> Result<Node, FormulaError> {
    let tokens = tokenize(src)?;
    let end = src.chars().count();
    if tokens.is_empty() {
        return Err(FormulaError::at("the formula is empty.", 0, 1));
    }
    let mut p = Parser {
        tokens,
        at: 0,
        depth: 0,
        end,
    };
    let node = p.expr()?;
    if let Some(t) = p.tokens.get(p.at) {
        return Err(FormulaError::at(
            format!(
                "unexpected {}; expected an operator or the end.",
                describe(&t.tok)
            ),
            t.pos,
            t.len,
        ));
    }
    Ok(node)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    depth: usize,
    /// Character count of the source: where "end of formula" errors point.
    end: usize,
}

fn describe(tok: &Tok) -> String {
    match tok {
        Tok::Num(_) => "number".to_owned(),
        Tok::Str(_) => "text".to_owned(),
        Tok::Name(n) => format!("'{}'", clip(n)),
        Tok::Bracket(n) => format!("[{}]", clip(n)),
        Tok::LParen => "'('".to_owned(),
        Tok::RParen => "')'".to_owned(),
        Tok::Comma => "','".to_owned(),
        Tok::Plus => "'+'".to_owned(),
        Tok::Minus => "'-'".to_owned(),
        Tok::Star => "'*'".to_owned(),
        Tok::Slash => "'/'".to_owned(),
        Tok::Eq => "'='".to_owned(),
        Tok::Ne => "'!='".to_owned(),
        Tok::Lt => "'<'".to_owned(),
        Tok::Le => "'<='".to_owned(),
        Tok::Gt => "'>'".to_owned(),
        Tok::Ge => "'>='".to_owned(),
    }
}

fn is_word(tok: &Tok, word: &str) -> bool {
    matches!(tok, Tok::Name(n) if n.eq_ignore_ascii_case(word))
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn peek_word(&self, word: &str) -> bool {
        self.peek().is_some_and(|t| is_word(&t.tok, word))
    }

    /// Position of the next token, or the end of the formula.
    fn here(&self) -> usize {
        self.peek().map_or(self.end, |t| t.pos)
    }

    /// Characters from `pos` to the end of the previous token.
    fn span_from(&self, pos: usize) -> usize {
        self.tokens
            .get(self.at.saturating_sub(1))
            .map_or(1, |t| t.pos + t.len - pos)
    }

    fn enter(&mut self) -> Result<(), FormulaError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(FormulaError::at(
                format!("the formula is nested more than {MAX_DEPTH} levels deep."),
                self.here(),
                1,
            ));
        }
        Ok(())
    }

    fn expr(&mut self) -> Result<Node, FormulaError> {
        self.enter()?;
        let node = self.or()?;
        self.depth -= 1;
        Ok(node)
    }

    fn binary(op: BinOp, left: Node, right: Node) -> Node {
        let pos = left.pos;
        let len = right.pos + right.len - pos;
        Node {
            kind: Kind::Bin(op, Box::new(left), Box::new(right)),
            pos,
            len,
        }
    }

    fn or(&mut self) -> Result<Node, FormulaError> {
        let mut left = self.and()?;
        while self.peek_word("or") {
            self.at += 1;
            let right = self.and()?;
            left = Self::binary(BinOp::Or, left, right);
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Node, FormulaError> {
        let mut left = self.not()?;
        while self.peek_word("and") {
            self.at += 1;
            let right = self.not()?;
            left = Self::binary(BinOp::And, left, right);
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<Node, FormulaError> {
        if self.peek_word("not") {
            let pos = self.here();
            self.at += 1;
            self.enter()?;
            let inner = self.not()?;
            self.depth -= 1;
            return Ok(Node {
                len: inner.pos + inner.len - pos,
                kind: Kind::Not(Box::new(inner)),
                pos,
            });
        }
        self.compare()
    }

    fn compare(&mut self) -> Result<Node, FormulaError> {
        let left = self.sum()?;
        let Some(op) = self.peek().and_then(|t| match t.tok {
            Tok::Eq => Some(BinOp::Eq),
            Tok::Ne => Some(BinOp::Ne),
            Tok::Lt => Some(BinOp::Lt),
            Tok::Le => Some(BinOp::Le),
            Tok::Gt => Some(BinOp::Gt),
            Tok::Ge => Some(BinOp::Ge),
            _ => None,
        }) else {
            return Ok(left);
        };
        self.at += 1;
        let right = self.sum()?;
        if let Some(t) = self.peek()
            && matches!(
                t.tok,
                Tok::Eq | Tok::Ne | Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge
            )
        {
            return Err(FormulaError::at(
                "comparisons do not chain; join them with 'and'.",
                t.pos,
                t.len,
            ));
        }
        Ok(Self::binary(op, left, right))
    }

    fn sum(&mut self) -> Result<Node, FormulaError> {
        let mut left = self.product()?;
        while let Some(op) = self.peek().and_then(|t| match t.tok {
            Tok::Plus => Some(BinOp::Add),
            Tok::Minus => Some(BinOp::Sub),
            _ => None,
        }) {
            self.at += 1;
            let right = self.product()?;
            left = Self::binary(op, left, right);
        }
        Ok(left)
    }

    fn product(&mut self) -> Result<Node, FormulaError> {
        let mut left = self.unary()?;
        while let Some(op) = self.peek().and_then(|t| match t.tok {
            Tok::Star => Some(BinOp::Mul),
            Tok::Slash => Some(BinOp::Div),
            _ => None,
        }) {
            self.at += 1;
            let right = self.unary()?;
            left = Self::binary(op, left, right);
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Node, FormulaError> {
        if self.peek().is_some_and(|t| t.tok == Tok::Minus) {
            let pos = self.here();
            self.at += 1;
            self.enter()?;
            let inner = self.unary()?;
            self.depth -= 1;
            return Ok(Node {
                len: inner.pos + inner.len - pos,
                kind: Kind::Neg(Box::new(inner)),
                pos,
            });
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Node, FormulaError> {
        let Some(token) = self.peek().cloned() else {
            return Err(FormulaError::at(
                "the formula ends where a value was expected.",
                self.end.saturating_sub(1),
                1,
            ));
        };
        self.at += 1;
        let leaf = |kind| {
            Ok(Node {
                kind,
                pos: token.pos,
                len: token.len,
            })
        };
        match token.tok {
            Tok::Num(v) => leaf(Kind::Num(v)),
            Tok::Str(s) => leaf(Kind::Str(s)),
            Tok::Bracket(name) => leaf(Kind::Name(name)),
            Tok::Name(name) => {
                if self.peek().is_some_and(|t| t.tok == Tok::LParen) {
                    return self.call(name, token.pos);
                }
                if name.eq_ignore_ascii_case("true") {
                    return leaf(Kind::Bool(true));
                }
                if name.eq_ignore_ascii_case("false") {
                    return leaf(Kind::Bool(false));
                }
                if ["and", "or", "not"]
                    .iter()
                    .any(|w| name.eq_ignore_ascii_case(w))
                {
                    return Err(FormulaError::at(
                        format!("'{name}' needs a value on each side."),
                        token.pos,
                        token.len,
                    ));
                }
                leaf(Kind::Name(name))
            }
            Tok::LParen => {
                let inner = self.expr()?;
                self.expect_close()?;
                Ok(Node {
                    kind: inner.kind,
                    pos: token.pos,
                    len: self.span_from(token.pos),
                })
            }
            other => Err(FormulaError::at(
                format!("unexpected {}; expected a value.", describe(&other)),
                token.pos,
                token.len,
            )),
        }
    }

    fn expect_close(&mut self) -> Result<(), FormulaError> {
        if self.peek().is_some_and(|t| t.tok == Tok::RParen) {
            self.at += 1;
            return Ok(());
        }
        let found = self
            .peek()
            .map_or_else(|| "the end of the formula".to_owned(), |t| describe(&t.tok));
        Err(FormulaError::at(
            format!("expected ')' but found {found}."),
            self.here().min(self.end.saturating_sub(1)),
            1,
        ))
    }

    fn call(&mut self, name: String, pos: usize) -> Result<Node, FormulaError> {
        self.at += 1; // the "("
        self.enter()?;
        let mut args = Vec::new();
        if !self.peek().is_some_and(|t| t.tok == Tok::RParen) {
            loop {
                args.push(self.expr()?);
                if self.peek().is_some_and(|t| t.tok == Tok::Comma) {
                    self.at += 1;
                    continue;
                }
                break;
            }
        }
        self.depth -= 1;
        self.expect_close()?;
        Ok(Node {
            kind: Kind::Call { name, args },
            pos,
            len: self.span_from(pos),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn shape(n: &Node) -> String {
        match &n.kind {
            Kind::Num(v) => format!("{v}"),
            Kind::Str(s) => format!("'{s}'"),
            Kind::Bool(b) => format!("{b}"),
            Kind::Name(n) => format!("<{n}>"),
            Kind::Call { name, args } => format!(
                "{name}({})",
                args.iter().map(shape).collect::<Vec<_>>().join(",")
            ),
            Kind::Neg(a) => format!("(-{})", shape(a)),
            Kind::Not(a) => format!("(not {})", shape(a)),
            Kind::Bin(op, a, b) => format!("({} {op:?} {})", shape(a), shape(b)),
        }
    }

    fn s(src: &str) -> String {
        shape(&parse(src).unwrap())
    }

    #[test]
    fn operators_bind_in_the_usual_order() {
        assert_eq!(s("1 + 2 * 3"), "(1 Add (2 Mul 3))");
        assert_eq!(s("1 - 2 - 3"), "((1 Sub 2) Sub 3)");
        assert_eq!(s("-a * b"), "((-<a>) Mul <b>)");
        assert_eq!(s("(1 + 2) * 3"), "((1 Add 2) Mul 3)");
        assert_eq!(
            s("a > 1 and b < 2 or not c = 3"),
            "(((<a> Gt 1) And (<b> Lt 2)) Or (not (<c> Eq 3)))"
        );
    }

    #[test]
    fn names_calls_brackets_and_booleans_parse() {
        assert_eq!(s("[Net Sales] / Sum(qty)"), "(<Net Sales> Div Sum(<qty>))");
        assert_eq!(s("If(TRUE, 'a', \"b\")"), "If(true,'a','b')");
        assert_eq!(s("Today()"), "Today()");
        assert_eq!(s("Round(x, 2)"), "Round(<x>,2)");
    }

    #[test]
    fn nodes_carry_the_span_they_cover() {
        let n = parse("  Sum([a]) + 1").unwrap();
        assert_eq!((n.pos, n.len), (2, 12));
        let Kind::Bin(_, l, r) = n.kind else {
            panic!("binary");
        };
        assert_eq!((l.pos, l.len), (2, 8));
        assert_eq!((r.pos, r.len), (13, 1));
        let n = parse("(1 + 2)").unwrap();
        assert_eq!((n.pos, n.len), (0, 7));
    }

    #[test]
    fn each_syntax_mistake_is_reported_at_its_character() {
        let at = |src: &str| parse(src).unwrap_err().position;
        assert_eq!(at(""), 0);
        assert_eq!(at("1 +"), 2, "ends where a value was expected");
        assert_eq!(at("(1 + 2"), 5, "missing close paren points at the end");
        assert_eq!(at("1 2"), 2);
        assert_eq!(at("Sum(a,)"), 6);
        assert_eq!(at("1 + * 2"), 4);
        assert_eq!(at("a < b < c"), 6, "comparisons do not chain");
        assert_eq!(at("and"), 0);
        assert_eq!(at("1 + )"), 4);
        assert_eq!(at("Sum(a b)"), 6);
    }

    #[test]
    fn nesting_past_the_limit_is_refused_without_overflowing() {
        let ok = format!("{}1{}", "(".repeat(30), ")".repeat(30));
        assert!(parse(&ok).is_ok());
        let deep = format!("{}1{}", "(".repeat(40), ")".repeat(40));
        let e = parse(&deep).unwrap_err();
        assert!(e.message.contains("32"), "{e:?}");
        let prefix = format!("{}1", "-".repeat(500));
        assert!(parse(&prefix).is_err());
        let nots = format!("{}true", "not ".repeat(300));
        assert!(parse(&nots).is_err());
        let calls = format!("{}1{}", "Abs(".repeat(100), ")".repeat(100));
        assert!(parse(&calls).is_err());
    }

    #[test]
    fn a_long_flat_chain_is_not_nesting() {
        let chain = "1+".repeat(900) + "1";
        let n = parse(&chain).unwrap();
        assert_eq!(n.len, chain.len());
    }
}
