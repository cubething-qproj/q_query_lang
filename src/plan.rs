//! Planning: resolve a parsed query against the type registry.
//!
//! A [`QueryPlan`] holds no World reference. All syntax and type-resolution
//! errors surface here; nothing runs.

use std::any::TypeId;

use bevy::{
    ecs::{name::Name, reflect::ReflectComponent},
    reflect::{TypeInfo, TypeRegistration, TypeRegistry},
};
use thiserror::Error;

use crate::{
    ast::*,
    parser::{ParseError, parse},
};

/// A parsed, type-checked query, ready to execute.
#[derive(Clone, Debug)]
pub struct QueryPlan {
    #[allow(dead_code, reason = "read by execution, phase 3")]
    pub(crate) stages: Vec<StagePlan>,
    schema: Schema,
    reads: Vec<TypeId>,
}

impl QueryPlan {
    /// Parses `text` and resolves its types against `registry`.
    pub fn new(text: &str, registry: &TypeRegistry) -> Result<Self, PlanError> {
        let program = parse(text)?;
        let types = RegisteredTypes::new(registry);
        let mut reads = Vec::new();
        let mut stages = Vec::with_capacity(program.stages.len());
        let mut schema = None;
        let mut opaque_fetch = None;
        for (index, stage) in program.stages.iter().enumerate() {
            let (planned, value, opaque) = plan_stage(stage, index == 0, &types, &mut reads)?;
            stages.push(planned);
            schema = Some(value.schema());
            opaque_fetch = opaque;
        }
        // Only the last stage's value is snapshotted, so only it may not be opaque.
        if let Some(error) = opaque_fetch {
            return Err(error);
        }
        reads.sort_unstable();
        reads.dedup();
        Ok(Self {
            stages,
            schema: schema.expect("the parser yields at least one stage"),
            reads,
        })
    }

    /// The shape of every result row, known before execution.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// The component types the plan reads, for scheduling.
    pub fn reads(&self) -> &[TypeId] {
        &self.reads
    }
}

/// The columns of a result row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Schema(pub Vec<Column>);

/// One result column.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Column {
    /// The term as written, without `?`; `@` for entities.
    pub label: String,
    /// The column's type.
    pub ty: ColumnType,
}

/// The type of a result column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColumnType {
    /// An entity id.
    Entity,
    /// A required component.
    Component(TypeId),
    /// An optional component.
    Optional(TypeId),
}

/// A planning error.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PlanError {
    /// The query is not syntactically valid.
    #[error(transparent)]
    Parse(#[from] ParseError),
    /// No reflect-registered type matches the path.
    #[error("unknown type `{ty}` at {span:?}: a typo, or a type that is not reflect-registered")]
    UnknownType {
        /// The type as written.
        ty: String,
        /// Where it was written.
        span: Span,
    },
    /// More than one registered type matches the path.
    #[error("ambiguous type `{ty}` at {span:?}; candidates: {}", candidates.join(", "))]
    AmbiguousType {
        /// The type as written.
        ty: String,
        /// Full paths of every matching type.
        candidates: Vec<String>,
        /// Where it was written.
        span: Span,
    },
    /// The type is registered but has no `ReflectComponent`.
    #[error("`{ty}` at {span:?} is not a component (no `#[reflect(Component)]`)")]
    NotAComponent {
        /// The type as written.
        ty: String,
        /// Where it was written.
        span: Span,
    },
    /// An opaque type is in the final, snapshotted data clause. Opaque types
    /// may still be filtered on, followed by a take, or fetched in earlier
    /// stages.
    #[error("`{ty}` at {span:?} is opaque and cannot be fetched; filter on it instead")]
    OpaqueFetch {
        /// The type as written.
        ty: String,
        /// Where it was written.
        span: Span,
    },
    /// A tuple names the same term twice.
    #[error("duplicate tuple term `{label}` at {span:?}")]
    DuplicateTerm {
        /// The repeated term.
        label: String,
        /// Where the repetition was written.
        span: Span,
    },
    /// `Entity` was used optionally or as a filter.
    #[error("at {span:?}: `Entity` can only be fetched, not made optional or filtered on")]
    ReservedEntity {
        /// Where it was written.
        span: Span,
    },
    /// An entity literal appeared after the first stage.
    #[error(
        "at {span:?}: entity literals may only begin queries and cannot appear in secondary stages."
    )]
    LiteralAfterFirstStage {
        /// Where it was written.
        span: Span,
    },
    /// A take was applied to something other than a single component.
    #[error("take at {span:?} needs a single relationship component; found {found}")]
    TakeOnNonComponent {
        /// What the take was applied to.
        found: &'static str,
        /// Where the take was written.
        span: Span,
    },
}

/// One planned stage.
#[derive(Clone, Debug)]
#[allow(dead_code, reason = "read by execution, phase 3")]
pub(crate) struct StagePlan {
    pub(crate) head: Head,
    pub(crate) ops: Vec<Op>,
}

/// What a stage produces before its postfix operators.
#[derive(Clone, Debug)]
#[allow(dead_code, reason = "read by execution, phase 3")]
pub(crate) enum Head {
    /// Entity literals; only in the first stage.
    Literal(Vec<EntityId>),
    /// A data clause: one term, or a tuple of two or more.
    Data(Vec<TermPlan>),
}

/// One resolved data term.
#[derive(Clone, Debug)]
#[allow(dead_code, reason = "read by execution, phase 3")]
pub(crate) enum TermPlan {
    Entity,
    Component { type_id: TypeId, optional: bool },
}

/// A resolved postfix operator.
#[derive(Clone, Debug)]
#[allow(dead_code, reason = "read by execution, phase 3")]
pub(crate) enum Op {
    Take(TakeRange),
    Select(FilterPlan),
}

/// A filter in negation normal form: negation appears only on atoms.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FilterPlan {
    With(TypeId),
    Without(TypeId),
    Name { name: String, negated: bool },
    And(Vec<FilterPlan>),
    Or(Vec<FilterPlan>),
}

/// The kind of value a stage's rows hold, tracked while planning.
enum Value {
    Entity,
    Component(Column),
    Tuple(Vec<Column>),
}

impl Value {
    fn schema(self) -> Schema {
        Schema(match self {
            Self::Entity => vec![entity_column()],
            Self::Component(column) => vec![column],
            Self::Tuple(columns) => columns,
        })
    }
}

fn entity_column() -> Column {
    Column {
        label: "@".to_owned(),
        ty: ColumnType::Entity,
    }
}

fn plan_stage(
    stage: &Stage,
    first: bool,
    types: &RegisteredTypes,
    reads: &mut Vec<TypeId>,
) -> Result<(StagePlan, Value, Option<PlanError>), PlanError> {
    let mut opaque = None;
    let mut ops = Vec::with_capacity(stage.postfix.len() + 1);
    let (head, mut value) = match &stage.primary {
        Primary::Literal { ids, span } => {
            if !first {
                return Err(PlanError::LiteralAfterFirstStage { span: span.clone() });
            }
            (Head::Literal(ids.clone()), Value::Entity)
        }
        // `#n` is sugar for `@[#n]`.
        Primary::Name(name) => {
            reads.push(TypeId::of::<Name>());
            ops.push(Op::Select(FilterPlan::Name {
                name: name.value.clone(),
                negated: false,
            }));
            (Head::Data(vec![TermPlan::Entity]), Value::Entity)
        }
        Primary::Data(data) => {
            let (head, value);
            (head, value, opaque) = plan_data(data, types, reads)?;
            (head, value)
        }
    };
    for postfix in &stage.postfix {
        match postfix {
            Postfix::Take { range, span } => {
                let found = match value {
                    Value::Component(_) => None,
                    Value::Entity => Some("an entity"),
                    Value::Tuple(_) => Some("a tuple"),
                };
                if let Some(found) = found {
                    return Err(PlanError::TakeOnNonComponent {
                        found,
                        span: span.clone(),
                    });
                }
                // Whether the component is a relationship is checked at execution.
                value = Value::Entity;
                opaque = None;
                ops.push(Op::Take(*range));
            }
            Postfix::Select { filter, .. } => {
                ops.push(Op::Select(plan_filter(filter, false, types, reads)?));
            }
        }
    }
    Ok((StagePlan { head, ops }, value, opaque))
}

fn plan_data(
    data: &Data,
    types: &RegisteredTypes,
    reads: &mut Vec<TypeId>,
) -> Result<(Head, Value, Option<PlanError>), PlanError> {
    // A one-element tuple is the same as its term.
    let terms: &[Term] = match data {
        Data::Term(term) => std::slice::from_ref(term),
        Data::Tuple { terms, .. } => terms,
    };
    let mut planned = Vec::with_capacity(terms.len());
    let mut columns: Vec<Column> = Vec::with_capacity(terms.len());
    let mut opaque = None;
    for term in terms {
        let (term_plan, column, span, opaque_fetch) = plan_term(term, types)?;
        opaque = opaque.or(opaque_fetch);
        if columns
            .iter()
            .any(|existing| same_term(existing.ty, column.ty))
        {
            return Err(PlanError::DuplicateTerm {
                label: column.label,
                span,
            });
        }
        if let TermPlan::Component { type_id, .. } = term_plan {
            reads.push(type_id);
        }
        planned.push(term_plan);
        columns.push(column);
    }
    let value = match <[Column; 1]>::try_from(columns) {
        Ok([column]) if column.ty == ColumnType::Entity => Value::Entity,
        Ok([column]) => Value::Component(column),
        Err(columns) => Value::Tuple(columns),
    };
    Ok((Head::Data(planned), value, opaque))
}

/// Resolves one data term. Also returns the error to raise if the term is
/// opaque and ends up snapshotted.
fn plan_term(
    term: &Term,
    types: &RegisteredTypes,
) -> Result<(TermPlan, Column, Span, Option<PlanError>), PlanError> {
    let (optional, ty, span) = match term {
        Term::Entity(span) => return Ok((TermPlan::Entity, entity_column(), span.clone(), None)),
        Term::Component { optional, ty, span } => (*optional, ty, span),
    };
    if is_entity(ty) {
        return if optional {
            Err(PlanError::ReservedEntity { span: span.clone() })
        } else {
            Ok((TermPlan::Entity, entity_column(), span.clone(), None))
        };
    }
    let registration = types.resolve(ty)?;
    let opaque_fetch =
        matches!(registration.type_info(), TypeInfo::Opaque(_)).then(|| PlanError::OpaqueFetch {
            ty: ty.to_string(),
            span: ty.span.clone(),
        });
    let type_id = registration.type_id();
    let column = Column {
        label: ty.to_string(),
        ty: if optional {
            ColumnType::Optional(type_id)
        } else {
            ColumnType::Component(type_id)
        },
    };
    Ok((
        TermPlan::Component { type_id, optional },
        column,
        span.clone(),
        opaque_fetch,
    ))
}

/// Whether two columns fetch the same thing, ignoring optionality.
fn same_term(a: ColumnType, b: ColumnType) -> bool {
    let id = |ty| match ty {
        ColumnType::Entity => None,
        ColumnType::Component(id) | ColumnType::Optional(id) => Some(id),
    };
    id(a) == id(b)
}

/// Resolves a filter and pushes negation inward (De Morgan).
fn plan_filter(
    filter: &Filter,
    negated: bool,
    types: &RegisteredTypes,
    reads: &mut Vec<TypeId>,
) -> Result<FilterPlan, PlanError> {
    let all = |operands: &[Filter], reads: &mut Vec<TypeId>| {
        operands
            .iter()
            .map(|operand| plan_filter(operand, negated, types, reads))
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(match &filter.kind {
        FilterKind::Has(ty) => {
            if is_entity(ty) {
                return Err(PlanError::ReservedEntity {
                    span: ty.span.clone(),
                });
            }
            let type_id = types.resolve(ty)?.type_id();
            if negated {
                FilterPlan::Without(type_id)
            } else {
                FilterPlan::With(type_id)
            }
        }
        FilterKind::Name(name) => {
            reads.push(TypeId::of::<Name>());
            FilterPlan::Name {
                name: name.value.clone(),
                negated,
            }
        }
        FilterKind::Not(operand) => plan_filter(operand, !negated, types, reads)?,
        FilterKind::And(operands) if negated => FilterPlan::Or(all(operands, reads)?),
        FilterKind::And(operands) => FilterPlan::And(all(operands, reads)?),
        FilterKind::Or(operands) if negated => FilterPlan::And(all(operands, reads)?),
        FilterKind::Or(operands) => FilterPlan::Or(all(operands, reads)?),
    })
}

/// The reserved builtin `Entity`, written bare.
fn is_entity(ty: &TypeAst) -> bool {
    ty.path == ["Entity"] && ty.args.is_empty()
}

/// Every registered type whose path parses, indexed for suffix matching.
struct RegisteredTypes<'r> {
    entries: Vec<(TypePath<'r>, &'r TypeRegistration)>,
}

impl<'r> RegisteredTypes<'r> {
    fn new(registry: &'r TypeRegistry) -> Self {
        Self {
            entries: registry
                .iter()
                .filter_map(|registration| {
                    TypePath::parse(registration.type_info().type_path())
                        .map(|path| (path, registration))
                })
                .collect(),
        }
    }

    /// Resolves `ty` to exactly one registered component.
    fn resolve(&self, ty: &TypeAst) -> Result<&'r TypeRegistration, PlanError> {
        let mut matches = self
            .entries
            .iter()
            .filter(|(path, _)| path.matches(ty))
            .map(|(_, registration)| *registration);
        let registration = match (matches.next(), matches.next()) {
            (None, _) => {
                return Err(PlanError::UnknownType {
                    ty: ty.to_string(),
                    span: ty.span.clone(),
                });
            }
            (Some(registration), None) => registration,
            (Some(first), Some(second)) => {
                let mut candidates: Vec<String> = [first, second]
                    .into_iter()
                    .chain(matches)
                    .map(|registration| registration.type_info().type_path().to_owned())
                    .collect();
                candidates.sort_unstable();
                return Err(PlanError::AmbiguousType {
                    ty: ty.to_string(),
                    candidates,
                    span: ty.span.clone(),
                });
            }
        };
        if registration.data::<ReflectComponent>().is_none() {
            return Err(PlanError::NotAComponent {
                ty: ty.to_string(),
                span: ty.span.clone(),
            });
        }
        Ok(registration)
    }
}

/// A registered type path, split into segments and generic arguments:
/// `a::B<c::D, E>`. Paths that are not of this shape (tuples, arrays,
/// references) are not queryable.
#[derive(Debug, Eq, PartialEq)]
struct TypePath<'a> {
    path: Vec<&'a str>,
    args: Vec<TypePath<'a>>,
}

impl<'a> TypePath<'a> {
    fn parse(text: &'a str) -> Option<Self> {
        let (path, rest) = Self::parse_prefix(text)?;
        rest.is_empty().then_some(path)
    }

    /// Parses one path from the start of `text`, returning the remainder.
    fn parse_prefix(text: &'a str) -> Option<(Self, &'a str)> {
        let end = text.find(['<', ',', '>']).unwrap_or(text.len());
        let (head, mut rest) = text.split_at(end);
        let path: Vec<&str> = head.split("::").collect();
        let is_identifier = |segment: &&str| {
            segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        };
        if path
            .iter()
            .any(|segment| segment.is_empty() || !is_identifier(segment))
        {
            return None;
        }
        let mut args = Vec::new();
        if let Some(inner) = rest.strip_prefix('<') {
            rest = inner;
            loop {
                let (arg, after) = Self::parse_prefix(rest.trim_start())?;
                args.push(arg);
                if let Some(after) = after.strip_prefix(',') {
                    rest = after;
                } else {
                    rest = after.strip_prefix('>')?;
                    break;
                }
            }
        }
        Some((Self { path, args }, rest))
    }

    /// Whether `ty`'s path is a segment-wise suffix of this one, with
    /// matching generic arguments.
    fn matches(&self, ty: &TypeAst) -> bool {
        self.path.len() >= ty.path.len()
            && self.path[self.path.len() - ty.path.len()..]
                .iter()
                .zip(&ty.path)
                .all(|(registered, written)| registered == written)
            && self.args.len() == ty.args.len()
            && self
                .args
                .iter()
                .zip(&ty.args)
                .all(|(arg, ty)| arg.matches(ty))
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use bevy::{prelude::*, reflect::TypeRegistry};

    use super::{FilterPlan, Op, QueryPlan, TypePath};

    #[derive(Component, Reflect)]
    #[reflect(Component)]
    struct A;

    #[derive(Component, Reflect)]
    #[reflect(Component)]
    struct B;

    /// The filter of `@[…]`, after normalization.
    fn filter(text: &str) -> FilterPlan {
        let mut registry = TypeRegistry::default();
        registry.register::<A>();
        registry.register::<B>();
        let plan = QueryPlan::new(&format!("@[{text}]"), &registry).unwrap();
        match plan.stages[0].ops.as_slice() {
            [Op::Select(filter)] => filter.clone(),
            other => panic!("{text:?}: {other:?}"),
        }
    }

    #[test]
    fn pushes_negation_to_the_atoms() {
        use FilterPlan::*;
        let (a, b) = (TypeId::of::<A>(), TypeId::of::<B>());
        let named = |negated| Name {
            name: "West".to_owned(),
            negated,
        };
        assert_eq!(filter("!(A B)"), Or(vec![Without(a), Without(b)]));
        assert_eq!(filter("!(A or B)"), And(vec![Without(a), Without(b)]));
        assert_eq!(filter("!!A"), With(a));
        assert_eq!(filter("!(A !#West)"), Or(vec![Without(a), named(false)]));
        assert_eq!(
            filter("A !(B or #West)"),
            And(vec![With(a), And(vec![Without(b), named(true)])])
        );
    }

    #[test]
    fn parses_registered_type_paths() {
        let path = TypePath::parse("a::B<c::D, E<F>>").unwrap();
        assert_eq!(path.path, ["a", "B"]);
        assert_eq!(path.args[0].path, ["c", "D"]);
        assert_eq!(path.args[1].args[0].path, ["F"]);
        for unqueryable in ["(f32, f32)", "[u8; 4]", "&str", "a::<B>", "a::B<C"] {
            assert_eq!(TypePath::parse(unqueryable), None, "{unqueryable:?}");
        }
    }
}
