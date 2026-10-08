use std::ops::Range;

use thiserror::Error;
use winnow::{
    ModalResult, Parser,
    combinator::{alt, cut_err, delimited, opt, preceded, repeat, separated, terminated},
    error::{ContextError, ErrMode},
    stream::LocatingSlice,
    token::{literal, take_while},
};

use crate::ast::*;

#[cfg(test)]
mod tests;

type Input<'a> = LocatingSlice<&'a str>;
type PResult<T> = ModalResult<T>;

/// A syntax error produced while parsing a query.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("could not parse query at byte {offset}: {message}")]
pub struct ParseError {
    /// Byte offset at which parsing failed.
    pub offset: usize,
    /// Parser diagnostic associated with the failure.
    pub message: String,
}

/// Parses one complete query with a hand-written implementation of the syntax
/// documented in `bnf/grammar.ebnf`.
pub fn parse(input: &str) -> Result<QueryAst<'_>, ParseError> {
    query.parse(Input::new(input)).map_err(|error| ParseError {
        offset: error.offset(),
        message: error.inner().to_string(),
    })
}

fn query<'a>(input: &mut Input<'a>) -> PResult<QueryAst<'a>> {
    separated(1.., term, literal("|"))
        .with_span()
        .map(|(terms, span)| QueryAst { terms, span })
        .parse_next(input)
}

fn term<'a>(input: &mut Input<'a>) -> PResult<TermAst<'a>> {
    alt((single, path.map(TermAst::Path))).parse_next(input)
}

fn single<'a>(input: &mut Input<'a>) -> PResult<TermAst<'a>> {
    delimited(literal("single("), query, literal(")"))
        .with_span()
        .map(|(query, span)| TermAst::Single {
            query: Box::new(query),
            span,
        })
        .parse_next(input)
}

fn path<'a>(input: &mut Input<'a>) -> PResult<PathAst<'a>> {
    (opt(separator), step, repeat(0.., (separator, step)))
        .with_span()
        .map(|((prefix, first, rest), span)| PathAst {
            prefix,
            first,
            rest,
            span,
        })
        .parse_next(input)
}

fn separator(input: &mut Input<'_>) -> PResult<PathSeparator> {
    alt((
        literal("//").value(PathSeparator::Descendants),
        literal("/").value(PathSeparator::Slash),
    ))
    .parse_next(input)
}

fn step<'a>(input: &mut Input<'a>) -> PResult<StepAst<'a>> {
    alt((parent_step, node_step, fetch_step)).parse_next(input)
}

fn parent_step<'a>(input: &mut Input<'a>) -> PResult<StepAst<'a>> {
    (literal(".."), repeat(0.., predicate))
        .with_span()
        .map(|((_, predicates), span)| StepAst::Parent { predicates, span })
        .parse_next(input)
}

fn node_step<'a>(input: &mut Input<'a>) -> PResult<StepAst<'a>> {
    (
        alt((
            (type_name, literal("::"), cut_err(node_test))
                .map(|(relationship, _, test)| (Some(relationship), test)),
            node_test.map(|test| (None, test)),
        )),
        repeat(0.., predicate),
    )
        .with_span()
        .map(|(((relationship, test), predicates), span)| {
            StepAst::Node(NodeStepAst {
                relationship,
                test,
                predicates,
                span,
            })
        })
        .parse_next(input)
}

fn node_test<'a>(input: &mut Input<'a>) -> PResult<NodeTest<'a>> {
    alt((name_test.map(NodeTest::Name), wildcard)).parse_next(input)
}

fn name_test<'a>(input: &mut Input<'a>) -> PResult<NameAst<'a>> {
    preceded(literal("#"), name).parse_next(input)
}

fn name<'a>(input: &mut Input<'a>) -> PResult<NameAst<'a>> {
    alt((
        quoted_name,
        identifier.with_span().map(|(value, span)| NameAst {
            value,
            quoted: false,
            span,
        }),
    ))
    .parse_next(input)
}

fn quoted_name<'a>(input: &mut Input<'a>) -> PResult<NameAst<'a>> {
    delimited(
        literal('"'),
        take_while(1.., |character: char| {
            character == ' ' || ('!'..='~').contains(&character) && character != '"'
        }),
        literal('"'),
    )
    .with_span()
    .map(|(value, span): (&'a str, Range<usize>)| NameAst {
        value,
        quoted: true,
        span,
    })
    .parse_next(input)
}

fn wildcard<'a>(input: &mut Input<'a>) -> PResult<NodeTest<'a>> {
    literal("*")
        .with_span()
        .map(|(_, span)| NodeTest::Wildcard(span))
        .parse_next(input)
}

fn fetch_step<'a>(input: &mut Input<'a>) -> PResult<StepAst<'a>> {
    (fetch, repeat(0.., predicate), repeat(0.., field))
        .with_span()
        .map(|((fetch, predicates, fields), span)| {
            StepAst::Fetch(FetchStepAst {
                fetch,
                predicates,
                fields,
                span,
            })
        })
        .parse_next(input)
}

fn fetch<'a>(input: &mut Input<'a>) -> PResult<FetchAst<'a>> {
    alt((
        optional_wrap,
        mut_wrap,
        ref_fetch,
        opt_mut_fetch,
        opt_fetch,
        mut_fetch,
        entity_fetch,
        component_fetch,
    ))
    .parse_next(input)
}

fn entity_fetch<'a>(input: &mut Input<'a>) -> PResult<FetchAst<'a>> {
    literal("@")
        .with_span()
        .map(|(_, span)| FetchAst::Entity { span })
        .parse_next(input)
}

fn component_fetch<'a>(input: &mut Input<'a>) -> PResult<FetchAst<'a>> {
    component(AccessMode::Read).parse_next(input)
}

fn opt_fetch<'a>(input: &mut Input<'a>) -> PResult<FetchAst<'a>> {
    access_fetch("?", AccessMode::OptionalRead).parse_next(input)
}

fn opt_mut_fetch<'a>(input: &mut Input<'a>) -> PResult<FetchAst<'a>> {
    access_fetch("?^", AccessMode::OptionalWrite).parse_next(input)
}

fn mut_fetch<'a>(input: &mut Input<'a>) -> PResult<FetchAst<'a>> {
    access_fetch("^", AccessMode::Write).parse_next(input)
}

fn optional_wrap<'a>(input: &mut Input<'a>) -> PResult<FetchAst<'a>> {
    delimited(
        literal("optional("),
        alt((
            delimited(literal("mut("), type_name, literal(")"))
                .map(|ty| (AccessMode::OptionalWrite, ty)),
            type_name.map(|ty| (AccessMode::OptionalRead, ty)),
        )),
        literal(")"),
    )
    .with_span()
    .map(|((access, ty), span)| FetchAst::Component { access, ty, span })
    .parse_next(input)
}

fn mut_wrap<'a>(input: &mut Input<'a>) -> PResult<FetchAst<'a>> {
    wrapped_fetch("mut(", AccessMode::Write).parse_next(input)
}

fn ref_fetch<'a>(input: &mut Input<'a>) -> PResult<FetchAst<'a>> {
    wrapped_fetch("ref(", AccessMode::Ref).parse_next(input)
}

fn component<'a>(
    access: AccessMode,
) -> impl Parser<Input<'a>, FetchAst<'a>, ErrMode<ContextError>> {
    type_name
        .with_span()
        .map(move |(ty, span)| FetchAst::Component { access, ty, span })
}

fn access_fetch<'a>(
    prefix: &'static str,
    access: AccessMode,
) -> impl Parser<Input<'a>, FetchAst<'a>, ErrMode<ContextError>> {
    preceded(literal(prefix), type_name)
        .with_span()
        .map(move |(ty, span)| FetchAst::Component { access, ty, span })
}

fn wrapped_fetch<'a>(
    prefix: &'static str,
    access: AccessMode,
) -> impl Parser<Input<'a>, FetchAst<'a>, ErrMode<ContextError>> {
    delimited(literal(prefix), type_name, literal(")"))
        .with_span()
        .map(move |(ty, span)| FetchAst::Component { access, ty, span })
}

fn predicate<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    preceded(literal("["), cut_err(terminated(pred_expr, literal("]")))).parse_next(input)
}

fn pred_expr<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    or_expr(input)
}

fn or_expr<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    (
        and_expr,
        repeat(
            0..,
            (spaces1, literal("or"), spaces1, and_expr).map(|(_, _, _, expression)| expression),
        ),
    )
        .with_span()
        .map(|((first, rest), span)| combine_predicates(first, rest, span, PredicateKind::Or))
        .parse_next(input)
}

fn and_expr<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    (
        pred_atom,
        repeat(0.., (and_separator, pred_atom).map(|(_, atom)| atom)),
    )
        .with_span()
        .map(|((first, rest), span)| combine_predicates(first, rest, span, PredicateKind::And))
        .parse_next(input)
}

fn and_separator(input: &mut Input<'_>) -> PResult<()> {
    alt(((spaces1, literal("and"), spaces1).void(), spaces1.void())).parse_next(input)
}

fn pred_atom<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    alt((
        not_group,
        pred_group,
        changed,
        added,
        spawned,
        with_predicate,
        without_predicate,
        presence,
        alt((last_predicate, position_predicate)),
    ))
    .parse_next(input)
}

fn presence<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    alt((
        preceded(literal("+"), type_name).map(PredicateKind::With),
        preceded(literal("-"), type_name).map(PredicateKind::Without),
    ))
    .with_span()
    .map(|(kind, span)| PredicateAst { kind, span })
    .parse_next(input)
}

fn with_predicate<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    wrapped_type("with(", PredicateKind::With).parse_next(input)
}

fn without_predicate<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    wrapped_type("without(", PredicateKind::Without).parse_next(input)
}

fn changed<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    wrapped_type("changed(", PredicateKind::Changed).parse_next(input)
}

fn added<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    wrapped_type("added(", PredicateKind::Added).parse_next(input)
}

fn wrapped_type<'a>(
    prefix: &'static str,
    constructor: fn(TypeName<'a>) -> PredicateKind<'a>,
) -> impl Parser<Input<'a>, PredicateAst<'a>, ErrMode<ContextError>> {
    delimited(literal(prefix), type_name, literal(")"))
        .map(constructor)
        .with_span()
        .map(|(kind, span)| PredicateAst { kind, span })
}

fn spawned<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    literal("spawned()")
        .with_span()
        .map(|(_, span)| PredicateAst {
            kind: PredicateKind::Spawned,
            span,
        })
        .parse_next(input)
}

fn last_predicate<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    literal("last()")
        .with_span()
        .map(|(_, span)| PredicateAst {
            kind: PredicateKind::Last,
            span,
        })
        .parse_next(input)
}

fn position_predicate<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    integer
        .with_span()
        .map(|(position, span)| PredicateAst {
            kind: PredicateKind::Position(position),
            span,
        })
        .parse_next(input)
}

fn not_group<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    delimited(literal("not("), pred_expr, literal(")"))
        .with_span()
        .map(|(predicate, span)| PredicateAst {
            kind: PredicateKind::Not(Box::new(predicate)),
            span,
        })
        .parse_next(input)
}

fn pred_group<'a>(input: &mut Input<'a>) -> PResult<PredicateAst<'a>> {
    delimited(literal("("), pred_expr, literal(")"))
        .with_span()
        .map(|(mut predicate, span)| {
            predicate.span = span;
            predicate
        })
        .parse_next(input)
}

fn combine_predicates<'a>(
    first: PredicateAst<'a>,
    rest: Vec<PredicateAst<'a>>,
    span: Range<usize>,
    constructor: fn(Vec<PredicateAst<'a>>) -> PredicateKind<'a>,
) -> PredicateAst<'a> {
    if rest.is_empty() {
        first
    } else {
        let mut predicates = Vec::with_capacity(rest.len() + 1);
        predicates.push(first);
        predicates.extend(rest);
        PredicateAst {
            kind: constructor(predicates),
            span,
        }
    }
}

fn field<'a>(input: &mut Input<'a>) -> PResult<FieldAst<'a>> {
    alt((field_member, field_position)).parse_next(input)
}

fn field_member<'a>(input: &mut Input<'a>) -> PResult<FieldAst<'a>> {
    preceded(
        literal("."),
        cut_err(alt((
            integer.map(FieldKind::Tuple),
            identifier.map(FieldKind::Named),
        ))),
    )
    .with_span()
    .map(|(kind, span)| FieldAst { kind, span })
    .parse_next(input)
}

fn field_position<'a>(input: &mut Input<'a>) -> PResult<FieldAst<'a>> {
    preceded(
        literal("["),
        cut_err(terminated(
            alt((
                literal("last()").value(FieldKind::Last),
                integer.map(FieldKind::Position),
            )),
            literal("]"),
        )),
    )
    .with_span()
    .map(|(kind, span)| FieldAst { kind, span })
    .parse_next(input)
}

fn type_name<'a>(input: &mut Input<'a>) -> PResult<TypeName<'a>> {
    identifier
        .with_span()
        .map(|(value, span)| TypeName { value, span })
        .parse_next(input)
}

fn identifier<'a>(input: &mut Input<'a>) -> PResult<&'a str> {
    (
        take_while(1, |character: char| {
            character.is_ascii_alphabetic() || character == '_'
        }),
        take_while(0.., |character: char| {
            character.is_ascii_alphanumeric() || character == '_'
        }),
    )
        .take()
        .parse_next(input)
}

fn integer(input: &mut Input<'_>) -> PResult<usize> {
    take_while(1.., |character: char| character.is_ascii_digit())
        .try_map(|digits: &str| digits.parse())
        .parse_next(input)
}

fn spaces1<'a>(input: &mut Input<'a>) -> PResult<&'a str> {
    take_while(1.., |character: char| character == ' ').parse_next(input)
}
