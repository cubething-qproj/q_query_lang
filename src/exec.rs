//! Execution: run a [`QueryPlan`] against a World and snapshot the result.
//!
//! Execution is atomic. Stages pass only entities to each other; only the
//! last stage's data is copied into the [`QueryResult`].

use std::fmt;

use bevy::{
    ecs::{
        component::ComponentId,
        entity::{EntityGeneration, EntityIndex},
        entity_disabling::DefaultQueryFilters,
        query::{QueryBuilder, QueryData, QueryFilter, QueryState},
        reflect::{AppTypeRegistry, ReflectComponent},
        relationship::RelationshipAccessor,
        world::FilteredEntityRef,
    },
    prelude::*,
};
use thiserror::Error;

use crate::{
    ast::{EntityId, TakeRange},
    plan::{FilterPlan, Head, Op, QueryPlan, Schema, StagePlan, TermPlan},
};

/// A snapshot of a query's result: owned rows plus the plan's schema.
#[derive(Debug)]
pub struct QueryResult {
    /// The shape of every row.
    pub schema: Schema,
    /// The rows, in result order.
    pub rows: Vec<Row>,
}

/// One result row.
#[derive(Debug)]
pub struct Row {
    /// The entity the row's data was read from.
    pub source: Entity,
    /// The row's data, shaped by the schema.
    pub value: RowValue,
}

/// A row's data.
#[derive(Debug)]
pub enum RowValue {
    /// An entity id (`@`, literals, `#n`, takes).
    Entity(Entity),
    /// A reflected copy of a component.
    Component(Box<dyn Reflect>),
    /// An optional component the entity lacks.
    Absent,
    /// A tuple, in schema order.
    Tuple(Vec<RowValue>),
}

/// An execution error: the query is valid, but the World disagrees with it.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ExecError {
    /// An entity literal names an entity that does not exist.
    #[error("entity {}v{} does not exist", entity.index, entity.generation)]
    StaleEntity {
        /// The literal as written.
        entity: EntityId,
    },
    /// An entity literal names an entity that default query filters exclude.
    #[error("entity {entity} is excluded by the default query filter `{filter}`")]
    FilteredEntity {
        /// The excluded entity.
        entity: Entity,
        /// The filtering component, e.g. `Disabled`.
        filter: String,
    },
    /// A take was applied to a component that is not a relationship.
    #[error("The take operator `[n]` can only apply to relationships. Found `{ty}`")]
    NotARelationship {
        /// The component as written.
        ty: String,
    },
    /// A component could not be copied into the result.
    #[error("Could not capture a snapshot of `{ty}`'s values: {reason}")]
    Snapshot {
        /// The component as written.
        ty: String,
        /// Why the reflected clone failed.
        reason: String,
    },
}

impl QueryPlan {
    /// Executes the plan atomically against `world`.
    ///
    /// Needs exclusive access because building dynamic `QueryState`s does.
    pub fn execute(&self, world: &mut World) -> Result<QueryResult, ExecError> {
        // Apply queued component registrations, so ids and relationship
        // accessors reflect every component the World knows.
        world.flush();
        let mut stages = self
            .stages
            .iter()
            .map(|stage| Stage::compile(stage, world))
            .collect::<Result<Vec<_>, _>>()?;
        let world: &World = world;
        let mut rows = Vec::new();
        for (index, stage) in stages.iter_mut().enumerate() {
            let input = (index > 0).then(|| rows.iter().map(|row: &RowRef| row.source).collect());
            rows = stage.run(input, world)?;
        }
        let terms = stages.last().map_or(&[][..], |stage| stage.terms());
        let rows = rows
            .into_iter()
            .map(|row| {
                Ok(Row {
                    source: row.source,
                    value: snapshot(row.value, terms, world)?,
                })
            })
            .collect::<Result<_, ExecError>>()?;
        Ok(QueryResult {
            schema: self.schema().clone(),
            rows,
        })
    }
}

/// A stage with its component ids resolved and its `QueryState` built.
struct Stage {
    source: Source,
    ops: Vec<CompiledOp>,
}

enum Source {
    Literal(Vec<EntityId>),
    Query {
        /// `None` when a required component is not initialized in this
        /// World: nothing can match.
        state: Option<Box<QueryState<FilteredEntityRef<'static, 'static>>>>,
        terms: Vec<Term>,
    },
}

/// A data term with its component id. `id` is `None` for components not yet
/// initialized in this World, which is ordinary absence.
enum Term {
    Entity,
    Component {
        id: Option<ComponentId>,
        optional: bool,
        label: String,
        reflect: ReflectComponent,
    },
}

enum CompiledOp {
    Take {
        range: TakeRange,
        /// `None` when the component is not initialized: nothing to follow.
        relationship: Option<(ComponentId, RelationshipAccessor)>,
    },
    Select(Filter),
}

/// A [`FilterPlan`] with component ids resolved.
enum Filter {
    With(Option<ComponentId>),
    Without(Option<ComponentId>),
    Name { name: String, negated: bool },
    And(Vec<Filter>),
    Or(Vec<Filter>),
}

/// A row before snapshotting: values are references into the World.
struct RowRef {
    source: Entity,
    value: RowValueRef,
}

/// A [`RowValue`] that still points into the World instead of owning a copy.
/// Stages pass these between operators; only the last stage's are turned
/// into [`RowValue`]s, by [`snapshot`].
enum RowValueRef {
    Entity(Entity),
    /// The component of `terms[term]` on `entity`.
    Component {
        entity: Entity,
        term: usize,
    },
    Absent,
    Tuple(Vec<RowValueRef>),
}

impl Stage {
    fn compile(plan: &StagePlan, world: &mut World) -> Result<Self, ExecError> {
        let mut source = match &plan.head {
            Head::Literal(ids) => Source::Literal(ids.clone()),
            Head::Data(terms) => {
                let terms: Vec<Term> = terms
                    .iter()
                    .map(|term| match term {
                        TermPlan::Entity => Term::Entity,
                        TermPlan::Component {
                            type_id,
                            optional,
                            label,
                            reflect,
                        } => Term::Component {
                            id: world.components().get_id(*type_id),
                            optional: *optional,
                            label: label.clone(),
                            reflect: reflect.clone(),
                        },
                    })
                    .collect();
                Source::Query { state: None, terms }
            }
        };
        let ops: Vec<CompiledOp> = plan
            .ops
            .iter()
            .map(|op| match op {
                Op::Select(filter) => Ok(CompiledOp::Select(Filter::compile(filter, world))),
                Op::Take(range) => {
                    // The plan only allows a take on a single component term.
                    let Source::Query { terms, .. } = &source else {
                        unreachable!("the plan rejects takes on literals")
                    };
                    let Some(Term::Component { id, label, .. }) = terms.first() else {
                        unreachable!("the plan rejects takes on entities")
                    };
                    let relationship = id
                        .map(|id| {
                            world
                                .components()
                                .get_info(id)
                                .and_then(|info| info.relationship_accessor())
                                .map(|accessor| (id, *accessor))
                                .ok_or_else(|| ExecError::NotARelationship { ty: label.clone() })
                        })
                        .transpose()?;
                    Ok(CompiledOp::Take {
                        range: *range,
                        relationship,
                    })
                }
            })
            .collect::<Result<_, _>>()?;
        if let Source::Query { state, terms } = &mut source {
            // A select directly after the data is part of the Bevy query.
            let leading = match ops.first() {
                Some(CompiledOp::Select(filter)) => Some(filter),
                _ => None,
            };
            *state = build_state(terms, leading, world);
        }
        Ok(Self { source, ops })
    }

    fn terms(&self) -> &[Term] {
        match &self.source {
            Source::Literal(_) => &[],
            Source::Query { terms, .. } => terms,
        }
    }

    /// Runs the stage over `input` (or the whole World), then its operators.
    fn run(&mut self, input: Option<Vec<Entity>>, world: &World) -> Result<Vec<RowRef>, ExecError> {
        let mut rows = match &mut self.source {
            Source::Literal(ids) => ids
                .iter()
                .map(|id| literal(*id, world))
                .collect::<Result<_, _>>()?,
            Source::Query { state: None, .. } => Vec::new(),
            Source::Query {
                state: Some(state),
                terms,
            } => {
                let row = |entity: FilteredEntityRef| row_ref(&entity, terms);
                match input {
                    None => state.iter(world).map(row).collect(),
                    Some(input) => state.iter_many(world, input).map(row).collect(),
                }
            }
        };
        for op in &self.ops {
            rows = match op {
                CompiledOp::Select(filter) => rows
                    .into_iter()
                    .filter(|row| filter.matches(row.source, world))
                    .collect(),
                CompiledOp::Take {
                    relationship: None, ..
                } => Vec::new(),
                CompiledOp::Take {
                    range,
                    relationship: Some((id, accessor)),
                } => rows
                    .iter()
                    .flat_map(|row| take(row, *id, accessor, *range, world))
                    .map(|entity| RowRef {
                        source: entity,
                        value: RowValueRef::Entity(entity),
                    })
                    .collect(),
            };
        }
        Ok(rows)
    }
}

fn build_state(
    terms: &[Term],
    leading: Option<&Filter>,
    world: &mut World,
) -> Option<Box<QueryState<FilteredEntityRef<'static, 'static>>>> {
    let mut builder = QueryBuilder::<FilteredEntityRef<'static, 'static>>::new(world);
    for term in terms {
        match term {
            Term::Entity => {}
            Term::Component {
                id: Some(id),
                optional: true,
                ..
            } => {
                let id = *id;
                builder.optional(move |builder| {
                    builder.ref_id(id);
                });
            }
            Term::Component {
                id: Some(id),
                optional: false,
                ..
            } => {
                builder.ref_id(*id);
            }
            Term::Component {
                id: None, optional, ..
            } => {
                if !optional {
                    return None;
                }
            }
        }
    }
    if leading.is_some_and(|filter| !filter.add_to(&mut builder)) {
        return None;
    }
    Some(Box::new(builder.build()))
}

fn row_ref(entity: &FilteredEntityRef, terms: &[Term]) -> RowRef {
    let id = entity.id();
    let mut values: Vec<RowValueRef> = terms
        .iter()
        .enumerate()
        .map(|(index, term)| match term {
            Term::Entity => RowValueRef::Entity(id),
            Term::Component { id: component, .. } => {
                if component.is_some_and(|component| entity.contains_id(component)) {
                    RowValueRef::Component {
                        entity: id,
                        term: index,
                    }
                } else {
                    RowValueRef::Absent
                }
            }
        })
        .collect();
    RowRef {
        source: id,
        value: if values.len() == 1 {
            values.remove(0)
        } else {
            RowValueRef::Tuple(values)
        },
    }
}

/// Resolves an entity literal, rejecting stale and default-filtered ids.
fn literal(id: EntityId, world: &World) -> Result<RowRef, ExecError> {
    let entity = EntityIndex::from_raw_u32(id.index)
        .map(|index| {
            Entity::from_index_and_generation(index, EntityGeneration::from_bits(id.generation))
        })
        .filter(|entity| world.get_entity(*entity).is_ok())
        .ok_or(ExecError::StaleEntity { entity: id })?;
    let filtered = world
        .get_resource::<DefaultQueryFilters>()
        .and_then(|filters| {
            filters
                .disabling_ids()
                .find(|filter| world.entity(entity).contains_id(*filter))
        });
    if let Some(filter) = filtered {
        return Err(ExecError::FilteredEntity {
            entity,
            filter: component_name(filter, world),
        });
    }
    Ok(RowRef {
        source: entity,
        value: RowValueRef::Entity(entity),
    })
}

/// A component's reflected type path, falling back to Bevy's debug name.
fn component_name(id: ComponentId, world: &World) -> String {
    let info = world.components().get_info(id);
    let type_path = info.and_then(|info| info.type_id()).and_then(|type_id| {
        let registry = world.get_resource::<AppTypeRegistry>()?.read();
        registry
            .get(type_id)
            .map(|registration| registration.type_info().type_path().to_owned())
    });
    type_path
        .unwrap_or_else(|| info.map_or_else(|| format!("{id:?}"), |info| info.name().to_string()))
}

/// The entities a row's relationship component points at, in relationship
/// order, narrowed to `range`.
///
/// Despawned targets are skipped: Bevy does not validate entries inserted
/// directly into a `RelationshipTarget`.
fn take(
    row: &RowRef,
    id: ComponentId,
    accessor: &RelationshipAccessor,
    range: TakeRange,
    world: &World,
) -> Vec<Entity> {
    // An optional relationship the entity lacks has nothing to follow.
    let RowValueRef::Component { entity, .. } = row.value else {
        return Vec::new();
    };
    let Some(ptr) = world.get_by_id(entity, id) else {
        return Vec::new();
    };
    let related: Vec<Entity> = match *accessor {
        RelationshipAccessor::Relationship {
            entity_field_offset,
            ..
        } => {
            // SAFETY: `ptr` points at component `id`, and `accessor` is that
            // component's own accessor, so `entity_field_offset` is the
            // `offset_of!` its `Entity` field: in bounds and aligned.
            vec![unsafe { *ptr.byte_add(entity_field_offset).deref::<Entity>() }]
        }
        RelationshipAccessor::RelationshipTarget { iter, .. } => {
            // SAFETY: `ptr` points at component `id`, and `accessor` is that
            // component's own accessor, which is the contract `iter` requires.
            unsafe { iter(ptr) }.collect()
        }
    };
    let related: Vec<Entity> = related
        .into_iter()
        .filter(|target| world.get_entity(*target).is_ok())
        .collect();
    let (start, end) = match range {
        TakeRange::Index(index) => (index, index.saturating_add(1)),
        TakeRange::Range {
            start,
            end,
            inclusive,
        } => (
            start.unwrap_or(0),
            end.map_or(related.len(), |end| {
                if inclusive {
                    end.saturating_add(1)
                } else {
                    end
                }
            }),
        ),
    };
    related
        .into_iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

impl Filter {
    fn compile(plan: &FilterPlan, world: &World) -> Self {
        let id = |type_id| world.components().get_id(type_id);
        let all = |plans: &[FilterPlan]| {
            plans
                .iter()
                .map(|plan| Self::compile(plan, world))
                .collect()
        };
        match plan {
            FilterPlan::With(type_id) => Self::With(id(*type_id)),
            FilterPlan::Without(type_id) => Self::Without(id(*type_id)),
            FilterPlan::Name { name, negated } => Self::Name {
                name: name.clone(),
                negated: *negated,
            },
            FilterPlan::And(plans) => Self::And(all(plans)),
            FilterPlan::Or(plans) => Self::Or(all(plans)),
        }
    }

    /// Adds what Bevy can express of this select to `builder`, so the built
    /// query matches `Query<D, F>` exactly, including which default query
    /// filters its access lifts (`[Disabled]`). The select still runs per row
    /// afterwards, so parts left out here (name tests, uninitialized
    /// components inside groups) stay correct. Returns `false` if the select
    /// can never match.
    fn add_to<D: QueryData, F: QueryFilter>(&self, builder: &mut QueryBuilder<D, F>) -> bool {
        match self {
            Self::With(None) => false,
            Self::Without(None) | Self::Name { .. } => true,
            Self::And(filters) => filters.iter().all(|filter| filter.add_to(builder)),
            exact if exact.is_exact() => {
                exact.add_exact(builder);
                true
            }
            Self::With(Some(_)) | Self::Without(Some(_)) | Self::Or(_) => true,
        }
    }

    /// Whether the builder can express this filter with no per-row parts.
    fn is_exact(&self) -> bool {
        match self {
            Self::With(id) | Self::Without(id) => id.is_some(),
            Self::Name { .. } => false,
            Self::And(filters) | Self::Or(filters) => filters.iter().all(Self::is_exact),
        }
    }

    /// Adds an [exact](Self::is_exact) filter, conjoined with what `builder`
    /// already holds.
    fn add_exact<D: QueryData, F: QueryFilter>(&self, builder: &mut QueryBuilder<D, F>) {
        match self {
            Self::With(Some(id)) => {
                builder.with_id(*id);
            }
            Self::Without(Some(id)) => {
                builder.without_id(*id);
            }
            Self::And(filters) => filters.iter().for_each(|filter| filter.add_exact(builder)),
            Self::Or(filters) => {
                builder.or(|builder| {
                    for filter in filters {
                        // Terms inside `or` are disjoined; group conjunctions.
                        match filter {
                            Self::And(_) => {
                                builder.and(|builder| filter.add_exact(builder));
                            }
                            _ => filter.add_exact(builder),
                        }
                    }
                });
            }
            Self::With(None) | Self::Without(None) | Self::Name { .. } => {
                unreachable!("only exact filters are added")
            }
        }
    }

    fn matches(&self, entity: Entity, world: &World) -> bool {
        let has =
            |id: Option<ComponentId>| id.is_some_and(|id| world.entity(entity).contains_id(id));
        match self {
            Self::With(id) => has(*id),
            Self::Without(id) => !has(*id),
            Self::Name { name, negated } => {
                world
                    .get::<Name>(entity)
                    .is_some_and(|actual| actual.as_str() == name)
                    != *negated
            }
            Self::And(filters) => filters.iter().all(|filter| filter.matches(entity, world)),
            Self::Or(filters) => filters.iter().any(|filter| filter.matches(entity, world)),
        }
    }
}

/// Copies a row's data out of the World.
fn snapshot(value: RowValueRef, terms: &[Term], world: &World) -> Result<RowValue, ExecError> {
    Ok(match value {
        RowValueRef::Entity(entity) => RowValue::Entity(entity),
        RowValueRef::Absent => RowValue::Absent,
        RowValueRef::Tuple(values) => RowValue::Tuple(
            values
                .into_iter()
                .map(|value| snapshot(value, terms, world))
                .collect::<Result<_, _>>()?,
        ),
        RowValueRef::Component { entity, term } => {
            let Term::Component { label, reflect, .. } = &terms[term] else {
                unreachable!("component values come from component terms")
            };
            let error = |reason: String| ExecError::Snapshot {
                ty: label.clone(),
                reason,
            };
            let component = reflect
                .reflect(world.entity(entity))
                .ok_or_else(|| error("the component is not readable".to_owned()))?;
            RowValue::Component(
                component
                    .reflect_clone()
                    .map_err(|e| error(e.to_string()))?,
            )
        }
    })
}

impl fmt::Display for QueryResult {
    /// One value per line; tuple results as a table with a header.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn text(value: &RowValue) -> String {
            match value {
                RowValue::Entity(entity) => entity.to_string(),
                RowValue::Component(component) => format!("{component:?}"),
                RowValue::Absent => "(none)".to_owned(),
                RowValue::Tuple(values) => {
                    let values: Vec<String> = values.iter().map(text).collect();
                    format!("({})", values.join(", "))
                }
            }
        }
        if self.schema.0.len() == 1 {
            return self
                .rows
                .iter()
                .try_for_each(|row| writeln!(f, "{}", text(&row.value)));
        }
        let mut lines: Vec<Vec<String>> =
            vec![self.schema.0.iter().map(|c| c.label.clone()).collect()];
        lines.extend(self.rows.iter().map(|row| match &row.value {
            RowValue::Tuple(values) => values.iter().map(text).collect(),
            single => vec![text(single)],
        }));
        let widths: Vec<usize> = (0..self.schema.0.len())
            .map(|column| {
                lines
                    .iter()
                    .filter_map(|line| line.get(column))
                    .map(String::len)
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        for line in &lines {
            let cells: Vec<String> = line
                .iter()
                .zip(&widths)
                .map(|(cell, width)| format!("{cell:<width$}"))
                .collect();
            writeln!(f, "{}", cells.join("  ").trim_end())?;
        }
        Ok(())
    }
}
