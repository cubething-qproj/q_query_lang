//! Syntax tree types produced by [`crate::parse`].

use std::ops::Range;

/// A byte range in the original query string.
pub type Span = Range<usize>;

/// A complete query, including every sequence-union term.
///
/// ABNF: `query = term *("|" term)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryAst<'a> {
    /// Terms joined by the `|` sequence-union operator.
    pub terms: Vec<TermAst<'a>>,
    /// The query's byte range.
    pub span: Span,
}

/// One path or `single(...)` term in a query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TermAst<'a> {
    /// A path expression.
    Path(PathAst<'a>),
    /// An exactly-one assertion around a nested query.
    Single {
        /// The nested query, boxed to break the recursive
        /// `QueryAst -> TermAst -> QueryAst` type cycle.
        query: Box<QueryAst<'a>>,
        /// The assertion's byte range, including `single(` and `)`.
        span: Span,
    },
}

/// An optional leading separator and one or more world steps.
///
/// ABNF: `path = [sep] step *(sep step)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathAst<'a> {
    /// The leading separator, or `None` for the implicit `//` world scan.
    pub prefix: Option<PathSeparator>,
    /// The first path step.
    pub first: StepAst<'a>,
    /// Remaining separator-step pairs.
    pub rest: Vec<(PathSeparator, StepAst<'a>)>,
    /// The path's byte range.
    pub span: Span,
}

/// A separator between path steps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathSeparator {
    /// `/`, a direct world step.
    Slash,
    /// `//`, a descendants-or-self step.
    Descendants,
}

/// One node-selection, parent, or data-fetch step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StepAst<'a> {
    /// An entity node test, optionally through an explicit relationship axis.
    Node(NodeStepAst<'a>),
    /// The `..` parent shorthand.
    Parent {
        /// Predicates attached to the selected parent nodes.
        predicates: Vec<PredicateAst<'a>>,
        /// The step's byte range.
        span: Span,
    },
    /// A `QueryData` fetch and its projections.
    Fetch(FetchStepAst<'a>),
}

/// An entity node test.
///
/// ABNF: `node-step = [rel-axis] node-test *predicate`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeStepAst<'a> {
    /// The relationship preceding `::`, or `None` for the default axis.
    pub relationship: Option<TypeName<'a>>,
    /// The name or wildcard test applied to related entities.
    pub test: NodeTest<'a>,
    /// Predicates attached to the selected entities.
    pub predicates: Vec<PredicateAst<'a>>,
    /// The step's byte range.
    pub span: Span,
}

/// A test applied to candidate entity nodes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NodeTest<'a> {
    /// `#Name`, including quoted names.
    Name(NameAst<'a>),
    /// `*`, with its source span.
    Wildcard(Span),
}

/// A data fetch with attached filters and reflected field projections.
///
/// ABNF: `fetch-step = fetch *predicate *field`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchStepAst<'a> {
    /// The requested entity or component data.
    pub fetch: FetchAst<'a>,
    /// Predicates evaluated against the source entity or result position.
    pub predicates: Vec<PredicateAst<'a>>,
    /// Reflected projections applied after fetching.
    pub fields: Vec<FieldAst<'a>>,
    /// The step's byte range.
    pub span: Span,
}

/// Data requested from each matching entity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FetchAst<'a> {
    /// The `@` entity-id fetch.
    Entity {
        /// The `@` byte range.
        span: Span,
    },
    /// A component fetch in one of the supported access modes.
    Component {
        /// Required, optional, mutable, presence, or change-aware access.
        access: AccessMode,
        /// The unresolved reflected component type name.
        ty: TypeName<'a>,
        /// The complete fetch byte range, including access syntax.
        span: Span,
    },
}

/// The `QueryData` access shape requested for a component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    /// `T` / `&T`.
    Read,
    /// `?T` / `Option<&T>`.
    OptionalRead,
    /// `^T` / `&mut T`.
    Write,
    /// `?^T` / `Option<&mut T>`.
    OptionalWrite,

    /// `ref(T)` / `Ref<T>`.
    Ref,
}

/// A predicate expression with its source range.
///
/// ABNF: `predicate = "[" pred-expr "]"`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PredicateAst<'a> {
    /// The parsed predicate operation.
    pub kind: PredicateKind<'a>,
    /// The expression's byte range.
    pub span: Span,
}

/// A presence, change-tick, positional, or logical predicate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PredicateKind<'a> {
    /// `+T` or `with(T)`.
    With(TypeName<'a>),
    /// `-T` or `without(T)`.
    Without(TypeName<'a>),
    /// `changed(T)`.
    Changed(TypeName<'a>),
    /// `added(T)`.
    Added(TypeName<'a>),
    /// `spawned()`.
    Spawned,
    /// A one-based numeric position.
    Position(usize),
    /// `last()`.
    Last,
    /// `not(...)`.
    Not(Box<PredicateAst<'a>>),
    /// A space- or `and`-joined conjunction.
    And(Vec<PredicateAst<'a>>),
    /// An `or`-joined disjunction.
    Or(Vec<PredicateAst<'a>>),
}

/// One reflected field or positional projection.
///
/// ABNF: `field = "." field-name / "[" positional "]"`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldAst<'a> {
    /// The parsed projection operation.
    pub kind: FieldKind<'a>,
    /// The projection's byte range, including punctuation.
    pub span: Span,
}

/// A reflected field or collection-position projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldKind<'a> {
    /// A named reflected field.
    Named(&'a str),
    /// A zero-based reflected tuple field such as `.0`.
    Tuple(usize),
    /// A one-based collection position such as `[1]`.
    Position(usize),
    /// The final collection position, `[last()]`.
    Last,
}

/// An unresolved reflected Rust type name.
///
/// ABNF: `type = IDENT`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypeName<'a> {
    /// The borrowed short type path.
    pub value: &'a str,
    /// The type name's byte range.
    pub span: Span,
}

/// An entity name test, with quotes removed from its borrowed value.
///
/// ABNF: `name = IDENT / quoted-name`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameAst<'a> {
    /// The borrowed entity name.
    pub value: &'a str,
    /// Whether the source used BSN-style quotes.
    pub quoted: bool,
    /// The name's byte range, including quotes when present.
    pub span: Span,
}
