use std::{any::TypeId, marker::PhantomData};

use bevy::{prelude::*, reflect::TypeRegistry};
use q_query_lang::{Column, ColumnType, PlanError, QueryPlan, Schema};

#[derive(Component, Reflect)]
#[reflect(Component)]
struct Health(u32);

#[derive(Component, Reflect)]
#[reflect(Component)]
struct Enemy;

#[derive(Component, Reflect, Clone)]
#[reflect(opaque, Component)]
struct Secret;

#[derive(Component, Reflect)]
#[reflect(Component)]
struct Wrapper<T: Send + Sync + TypePath>(#[reflect(ignore)] PhantomData<T>);

mod a {
    use bevy::prelude::*;

    #[derive(Component, Reflect)]
    #[reflect(Component)]
    pub struct Dup;
}

mod b {
    use bevy::prelude::*;

    #[derive(Component, Reflect)]
    #[reflect(Component)]
    pub struct Dup;
}

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    registry.register::<Name>();
    registry.register::<Children>();
    registry.register::<ChildOf>();
    registry.register::<Vec3>();
    registry.register::<Health>();
    registry.register::<Enemy>();
    registry.register::<Secret>();
    registry.register::<Wrapper<Enemy>>();
    registry.register::<a::Dup>();
    registry.register::<b::Dup>();
    registry
}

/// Plans `text`, logging the schema or error. See it with `--no-capture`.
fn plan(text: &str) -> Result<QueryPlan, PlanError> {
    let plan = QueryPlan::new(text, &registry());
    match &plan {
        Ok(plan) => println!(
            "> {text}\nschema: {:?}\nreads: {:?}\n",
            plan.schema(),
            plan.reads()
        ),
        Err(error) => println!("> {text}\nerror: {error}\n"),
    }
    plan
}

fn schema(text: &str) -> Schema {
    plan(text).unwrap().schema().clone()
}

fn column(label: &str, ty: ColumnType) -> Column {
    Column {
        label: label.to_owned(),
        ty,
    }
}

fn entity() -> Column {
    column("@", ColumnType::Entity)
}

#[test]
fn resolves_types_by_path_suffix() {
    let health = ColumnType::Component(TypeId::of::<Health>());
    assert_eq!(schema("Health"), Schema(vec![column("Health", health)]));
    assert_eq!(
        schema("plan::Health"),
        Schema(vec![column("plan::Health", health)])
    );
    assert_eq!(
        schema("a::Dup"),
        Schema(vec![column(
            "a::Dup",
            ColumnType::Component(TypeId::of::<a::Dup>())
        )])
    );
    // Matching is by whole path segments, not by string suffix.
    for partial in ["ealth", "lan::Health"] {
        assert!(
            matches!(plan(partial), Err(PlanError::UnknownType { .. })),
            "{partial:?}"
        );
    }
}

#[test]
fn resolves_generic_types_structurally() {
    let wrapper = ColumnType::Component(TypeId::of::<Wrapper<Enemy>>());
    assert_eq!(
        schema("Wrapper<Enemy>"),
        Schema(vec![column("Wrapper<Enemy>", wrapper)])
    );
    assert!(matches!(
        plan("Wrapper"),
        Err(PlanError::UnknownType { .. })
    ));
    assert!(matches!(
        plan("Wrapper<Health>"),
        Err(PlanError::UnknownType { .. })
    ));
}

#[test]
fn schema_follows_the_last_data_clause() {
    let name = ColumnType::Component(TypeId::of::<Name>());
    let health = ColumnType::Optional(TypeId::of::<Health>());
    assert_eq!(
        schema("(@ Name ?Health)[Enemy]"),
        Schema(vec![
            entity(),
            column("Name", name),
            column("Health", health)
        ])
    );
    // A one-element tuple is its term; entities are always `@`.
    assert_eq!(schema("(Name)"), Schema(vec![column("Name", name)]));
    for entities in [
        "@[Enemy]",
        "Entity",
        "#Floor",
        "@12v3",
        "#Floor | Children[..]",
    ] {
        assert_eq!(schema(entities), Schema(vec![entity()]), "{entities:?}");
    }
    assert_eq!(schema("Health | Name"), Schema(vec![column("Name", name)]));
}

#[test]
fn records_reads_but_not_filters() {
    let reads = |text: &str| plan(text).unwrap().reads().to_vec();
    let mut expected = vec![TypeId::of::<Name>(), TypeId::of::<Health>()];
    expected.sort_unstable();
    assert_eq!(reads("(Name ?Health)[Enemy !Secret]"), expected);
    assert_eq!(reads("#Floor"), [TypeId::of::<Name>()]);
    assert_eq!(reads("@[Enemy]"), []);
}

#[test]
fn rejects_invalid_plans() {
    let cases: &[(&str, fn(&PlanError) -> bool)] = &[
        ("Foo[", |e| matches!(e, PlanError::Parse(_))),
        ("Fooo", |e| matches!(e, PlanError::UnknownType { .. })),
        ("Vec3", |e| matches!(e, PlanError::NotAComponent { .. })),
        ("Secret", |e| matches!(e, PlanError::OpaqueFetch { .. })),
        ("?Secret", |e| matches!(e, PlanError::OpaqueFetch { .. })),
        ("(Name Secret)", |e| {
            matches!(e, PlanError::OpaqueFetch { .. })
        }),
        ("Name | Secret", |e| {
            matches!(e, PlanError::OpaqueFetch { .. })
        }),
        ("(Name Name)", |e| {
            matches!(e, PlanError::DuplicateTerm { .. })
        }),
        ("(Health ?plan::Health)", |e| {
            matches!(e, PlanError::DuplicateTerm { .. })
        }),
        ("(@ Entity)", |e| {
            matches!(e, PlanError::DuplicateTerm { .. })
        }),
        ("?Entity", |e| matches!(e, PlanError::ReservedEntity { .. })),
        ("Name[Entity]", |e| {
            matches!(e, PlanError::ReservedEntity { .. })
        }),
        ("Name | @12v3", |e| {
            matches!(e, PlanError::LiteralAfterFirstStage { .. })
        }),
        ("(Name Health)[0]", |e| {
            matches!(e, PlanError::TakeOnNonComponent { .. })
        }),
        ("#Floor[..]", |e| {
            matches!(e, PlanError::TakeOnNonComponent { .. })
        }),
        ("@12v3[0]", |e| {
            matches!(e, PlanError::TakeOnNonComponent { .. })
        }),
        ("Children[..][0]", |e| {
            matches!(e, PlanError::TakeOnNonComponent { .. })
        }),
        ("Name[Fooo]", |e| matches!(e, PlanError::UnknownType { .. })),
    ];
    for (text, is_expected) in cases {
        let error = plan(text).unwrap_err();
        assert!(is_expected(&error), "{text:?}: {error:?}");
    }
}

#[test]
fn ambiguous_types_list_every_candidate() {
    let Err(PlanError::AmbiguousType { candidates, .. }) = plan("Dup") else {
        panic!("expected an ambiguity");
    };
    assert_eq!(candidates, ["plan::a::Dup", "plan::b::Dup"]);
}

#[test]
fn opaque_types_are_usable_unless_snapshotted() {
    // Filters, earlier stages, and takes never snapshot the opaque value.
    for text in ["@[Secret]", "Name[!Secret]", "Secret | Name", "Secret[..]"] {
        assert!(plan(text).is_ok(), "{text:?}: {:?}", plan(text));
    }
}
