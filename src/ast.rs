//! Owned syntax tree produced by [`crate::parse`].
//!
//! Every node carries the byte span of its source text. The tree owns its
//! strings so that plans never borrow the query text.

use std::ops::Range;

/// A byte range in the original query string.
pub type Span = Range<usize>;

/// A complete query: stages joined by `|`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Program {
    /// The stages, in evaluation order. Min length of 1.
    pub stages: Vec<Stage>,
    /// The program's byte range.
    pub span: Span,
}

/// One pipeline stage: a primary followed by postfix operators.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stage {
    /// What the stage selects or fetches.
    pub primary: Primary,
    /// Takes and selects, applied left to right.
    pub postfix: Vec<Postfix>,
    /// The stage's byte range.
    pub span: Span,
}

/// The head of a stage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Primary {
    /// `@12v3` or `@(12v3 4v1)`.
    Literal {
        /// The entity ids, in written order. Min length of 1.
        ids: Vec<EntityId>,
        /// The literal's byte range.
        span: Span,
    },
    /// `#n`, sugar for `@[#n]`.
    Name(NameAst),
    /// A data clause.
    Data(Data),
}

/// An entity id written in Bevy's `Entity` `Display` form, `{index}v{generation}`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EntityId {
    /// The entity index.
    pub index: u32,
    /// The entity generation.
    pub generation: u32,
}

/// The data clause of a stage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Data {
    /// A single term, e.g. `Name`.
    Term(Term),
    /// A parenthesized tuple of terms, e.g. `(Name ?Health)`.
    Tuple {
        /// The terms, in written order. Min length of 1.
        terms: Vec<Term>,
        /// The tuple's byte range, including parentheses.
        span: Span,
    },
}

/// One data term.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Term {
    /// `@`, sugar for `Entity`.
    Entity(Span),
    /// `T` or `?T`.
    Component {
        /// Whether the term was written `?T`.
        optional: bool,
        /// The component type.
        ty: TypeAst,
        /// The term's byte range, including `?`.
        span: Span,
    },
}

/// A postfix operator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Postfix {
    /// `[n]` or a range: follow a relationship.
    Take {
        /// Which related entities to follow.
        range: TakeRange,
        /// The operator's byte range, including brackets.
        span: Span,
    },
    /// `[F]`: keep rows whose source entity satisfies the filter.
    Select {
        /// The filter expression.
        filter: Filter,
        /// The operator's byte range, including brackets.
        span: Span,
    },
}

/// The positions selected by a take.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TakeRange {
    /// `[n]`.
    Index(usize),
    /// `[a..b]`, `[a..=b]`, and their open-ended forms.
    Range {
        /// The first position, or `None` for the start.
        start: Option<usize>,
        /// The end position, or `None` for the end.
        end: Option<usize>,
        /// Whether `end` is included (`..=`). Implies `end` is `Some`.
        inclusive: bool,
    },
}

/// A filter expression, before normalization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Filter {
    /// The expression.
    pub kind: FilterKind,
    /// The expression's byte range.
    pub span: Span,
}

/// A filter operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FilterKind {
    /// `T`: the entity has `T`.
    Has(TypeAst),
    /// `#n`: the entity's `Name` is `n`.
    Name(NameAst),
    /// `!F`.
    Not(Box<Filter>),
    /// Juxtaposition. Min length of 2.
    And(Vec<Filter>),
    /// `or`. Min length of 2.
    Or(Vec<Filter>),
}

/// A type, possibly path-qualified and generic: `my_crate::Foo<Bar>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypeAst {
    /// The `::`-separated path segments. Min length of 1.
    pub path: Vec<String>,
    /// Generic arguments, empty for non-generic types.
    pub args: Vec<TypeAst>,
    /// The type's byte range.
    pub span: Span,
}

/// An entity name, with quotes removed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameAst {
    /// The name.
    pub value: String,
    /// Whether the source used quotes.
    pub quoted: bool,
    /// The name's byte range, including quotes.
    pub span: Span,
}
