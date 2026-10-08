//! Hand-written winnow parser for the grammar in the q_query_lang design doc.
//!
//! Every token parser skips leading whitespace, so whitespace may appear
//! between any two tokens. Node spans start at the node's first token.

use std::{fmt, str::FromStr};

use thiserror::Error;
use winnow::{
    Parser,
    ascii::multispace0,
    combinator::{alt, cut_err, eof, opt, peek, preceded, repeat, terminated},
    error::{ContextError, ErrMode},
    stream::LocatingSlice,
    token::{literal, one_of, take_while},
};

use crate::ast::*;

#[cfg(test)]
mod tests;

type Input<'a> = LocatingSlice<&'a str>;
type Error = ErrMode<ContextError<Expected>>;
type PResult<T> = Result<T, Error>;

/// A syntax error: the parser needed `expected` at byte `offset`.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("expected {expected} at byte {offset}")]
pub struct ParseError {
    /// What the parser needed.
    pub expected: Expected,
    /// Byte offset at which parsing failed.
    pub offset: usize,
}

/// What the parser expected where it failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Expected {
    /// A type: `Foo`, `my_crate::Foo<Bar>`.
    Type,
    /// An entity name: `West`, `"Floor West"`.
    Name,
    /// An entity id: `12v3`.
    EntityId,
    /// A position in a take: `2`.
    Integer,
    /// A closing `)`.
    CloseParen,
    /// A closing `]`.
    CloseBracket,
    /// A closing `>`.
    CloseAngle,
    /// `|`, a postfix operator, or the end of the query.
    EndOfStage,
}

impl fmt::Display for Expected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Type => "a type",
            Self::Name => "a name",
            Self::EntityId => "an entity id",
            Self::Integer => "an integer",
            Self::CloseParen => "`)`",
            Self::CloseBracket => "`]`",
            Self::CloseAngle => "`>`",
            Self::EndOfStage => "`|`, `[`, or the end of the query",
        })
    }
}

/// Parses one complete query.
pub fn parse(input: &str) -> Result<Program, ParseError> {
    terminated(program, (multispace0, eof.context(Expected::EndOfStage)))
        .parse(Input::new(input))
        .map_err(|error| ParseError {
            // Contexts accumulate while unwinding; the innermost is the most
            // specific. Every failure path is labelled; the negative corpus
            // tests exercise them.
            expected: *error
                .inner()
                .context()
                .next()
                .expect("every parse failure carries an `Expected` context"),
            offset: error.offset(),
        })
}

fn program(input: &mut Input<'_>) -> PResult<Program> {
    spanned(at_least(stage, preceded(token("|"), cut_err(stage))))
        .map(|(stages, span)| Program { stages, span })
        .parse_next(input)
}

fn stage(input: &mut Input<'_>) -> PResult<Stage> {
    spanned((primary, repeat(0.., postfix)))
        .map(|((primary, postfix), span)| Stage {
            primary,
            postfix,
            span,
        })
        .parse_next(input)
}

fn primary(input: &mut Input<'_>) -> PResult<Primary> {
    alt((
        entity_literal,
        name_selector.map(Primary::Name),
        data.map(Primary::Data),
    ))
    .parse_next(input)
}

fn entity_literal(input: &mut Input<'_>) -> PResult<Primary> {
    spanned(preceded(
        token("@"),
        alt((
            entity_id.map(|id| vec![id]),
            preceded(
                token("("),
                cut_err(terminated(
                    at_least(entity_id, entity_id),
                    closer(")", Expected::CloseParen),
                )),
            ),
        )),
    ))
    .map(|(ids, span)| Primary::Literal { ids, span })
    .parse_next(input)
}

fn entity_id(input: &mut Input<'_>) -> PResult<EntityId> {
    preceded(
        multispace0,
        (
            digits(Expected::EntityId),
            cut_err(preceded(literal("v"), digits(Expected::EntityId))),
        ),
    )
    .map(|(index, generation)| EntityId { index, generation })
    .parse_next(input)
}

fn name_selector(input: &mut Input<'_>) -> PResult<NameAst> {
    preceded(token("#"), cut_err(name)).parse_next(input)
}

fn data(input: &mut Input<'_>) -> PResult<Data> {
    alt((
        spanned(preceded(
            token("("),
            cut_err(terminated(
                at_least(term, term),
                closer(")", Expected::CloseParen),
            )),
        ))
        .map(|(terms, span)| Data::Tuple { terms, span }),
        term.map(Data::Term),
    ))
    .parse_next(input)
}

fn term(input: &mut Input<'_>) -> PResult<Term> {
    alt((
        spanned(token("@")).map(|(_, span)| Term::Entity(span)),
        spanned(alt((
            preceded(token("?"), cut_err(type_ast)).map(|ty| (true, ty)),
            type_ast.map(|ty| (false, ty)),
        )))
        .map(|((optional, ty), span)| Term::Component { optional, ty, span }),
    ))
    .parse_next(input)
}

fn postfix(input: &mut Input<'_>) -> PResult<Postfix> {
    enum Inner {
        Take(TakeRange),
        Select(Filter),
    }
    spanned(preceded(
        token("["),
        cut_err(terminated(
            alt((take_range.map(Inner::Take), filter.map(Inner::Select))),
            closer("]", Expected::CloseBracket),
        )),
    ))
    .map(|(inner, span)| match inner {
        Inner::Take(range) => Postfix::Take { range, span },
        Inner::Select(filter) => Postfix::Select { filter, span },
    })
    .parse_next(input)
}

fn take_range(input: &mut Input<'_>) -> PResult<TakeRange> {
    alt((
        (opt(int), token("..="), cut_err(int)).map(|(start, _, end)| TakeRange::Range {
            start,
            end: Some(end),
            inclusive: true,
        }),
        (opt(int), token(".."), opt(int)).map(|(start, _, end)| TakeRange::Range {
            start,
            end,
            inclusive: false,
        }),
        int.map(TakeRange::Index),
    ))
    .parse_next(input)
}

fn filter(input: &mut Input<'_>) -> PResult<Filter> {
    spanned(at_least(or_expr, or_expr))
        .map(|(operands, span)| combine(operands, span, FilterKind::And))
        .parse_next(input)
}

fn or_expr(input: &mut Input<'_>) -> PResult<Filter> {
    spanned(at_least(unary, preceded(keyword_or, cut_err(unary))))
        .map(|(operands, span)| combine(operands, span, FilterKind::Or))
        .parse_next(input)
}

fn unary(input: &mut Input<'_>) -> PResult<Filter> {
    alt((
        spanned(preceded(token("!"), cut_err(unary))).map(|(operand, span)| Filter {
            kind: FilterKind::Not(Box::new(operand)),
            span,
        }),
        atom,
    ))
    .parse_next(input)
}

fn atom(input: &mut Input<'_>) -> PResult<Filter> {
    alt((
        spanned(preceded(
            token("("),
            cut_err(terminated(filter, closer(")", Expected::CloseParen))),
        ))
        .map(|(inner, span)| Filter {
            kind: inner.kind,
            span,
        }),
        spanned(name_selector).map(|(name, span)| Filter {
            kind: FilterKind::Name(name),
            span,
        }),
        // `or` is reserved here, so no filter path may start with it.
        type_ast
            .verify(|ty: &TypeAst| ty.path[0] != "or")
            .context(Expected::Type)
            .map(|ty| Filter {
                span: ty.span.clone(),
                kind: FilterKind::Has(ty),
            }),
    ))
    .parse_next(input)
}

/// Wraps two or more operands in `kind`; a single operand stands alone.
fn combine(mut operands: Vec<Filter>, span: Span, kind: fn(Vec<Filter>) -> FilterKind) -> Filter {
    if operands.len() == 1 {
        operands.remove(0)
    } else {
        Filter {
            kind: kind(operands),
            span,
        }
    }
}

fn type_ast(input: &mut Input<'_>) -> PResult<TypeAst> {
    spanned((
        at_least(identifier, preceded(token("::"), cut_err(identifier))),
        opt(preceded(
            token("<"),
            cut_err(terminated(
                at_least(type_ast, type_ast),
                closer(">", Expected::CloseAngle),
            )),
        )),
    ))
    .map(|((path, args), span)| TypeAst {
        path: path.into_iter().map(String::from).collect(),
        args: args.unwrap_or_default(),
        span,
    })
    .context(Expected::Type)
    .parse_next(input)
}

fn name(input: &mut Input<'_>) -> PResult<NameAst> {
    alt((
        spanned(preceded(
            literal('"'),
            cut_err(terminated(
                take_while(1.., |c: char| {
                    c == ' ' || c == '!' || ('#'..='~').contains(&c)
                }),
                literal('"'),
            )),
        ))
        .map(|(value, span): (&str, Span)| NameAst {
            value: value.to_owned(),
            quoted: true,
            span,
        }),
        spanned(identifier).map(|(value, span)| NameAst {
            value: value.to_owned(),
            quoted: false,
            span,
        }),
    ))
    .context(Expected::Name)
    .parse_next(input)
}

fn keyword_or(input: &mut Input<'_>) -> PResult<()> {
    identifier
        .verify(|word: &str| word == "or")
        .void()
        .parse_next(input)
}

fn identifier<'a>(input: &mut Input<'a>) -> PResult<&'a str> {
    preceded(
        multispace0,
        (
            take_while(1, |c: char| c.is_ascii_alphabetic() || c == '_'),
            take_while(0.., |c: char| c.is_ascii_alphanumeric() || c == '_'),
        )
            .take(),
    )
    .parse_next(input)
}

fn int(input: &mut Input<'_>) -> PResult<usize> {
    preceded(multispace0, digits(Expected::Integer)).parse_next(input)
}

/// Decimal digits parsed as `T`. Once a digit is seen, overflow is fatal:
/// nothing else in the grammar starts with a digit.
fn digits<'a, T>(expected: Expected) -> impl Parser<Input<'a>, T, Error>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    preceded(
        peek(one_of(|c: char| c.is_ascii_digit())),
        cut_err(take_while(1.., |c: char| c.is_ascii_digit()).try_map(str::parse::<T>)),
    )
    .context(expected)
}

/// `first`, then any number of `rest`.
fn at_least<'a, O>(
    first: impl Parser<Input<'a>, O, Error>,
    rest: impl Parser<Input<'a>, O, Error>,
) -> impl Parser<Input<'a>, Vec<O>, Error> {
    (first, repeat(0.., rest)).map(|(first, rest): (O, Vec<O>)| {
        let mut operands = vec![first];
        operands.extend(rest);
        operands
    })
}

fn token<'a>(text: &'static str) -> impl Parser<Input<'a>, &'a str, Error> {
    preceded(multispace0, literal(text))
}

/// A closing delimiter, labelled so errors name it.
fn closer<'a>(text: &'static str, expected: Expected) -> impl Parser<Input<'a>, &'a str, Error> {
    token(text).context(expected)
}

/// Skips leading whitespace, then records the span of `parser`'s match.
fn spanned<'a, O>(
    parser: impl Parser<Input<'a>, O, Error>,
) -> impl Parser<Input<'a>, (O, Span), Error> {
    preceded(multispace0, parser.with_span())
}
