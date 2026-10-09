use bevy::{ecs::entity_disabling::Disabled, prelude::*};
use q_query_lang::{ExecError, QueryPlan, QueryResult, RowValue};

#[derive(Component, Reflect, Debug, PartialEq)]
#[reflect(Component)]
struct Health(u32);

#[derive(Component, Reflect)]
#[reflect(Component)]
struct Enemy;

#[derive(Component, Reflect)]
#[reflect(Component)]
struct Dead;

/// Registered for reflection but never spawned.
#[derive(Component, Reflect)]
#[reflect(Component)]
struct Unspawned;

#[derive(Component, Reflect)]
#[reflect(Component)]
#[relationship(relationship_target = DockedBy)]
struct DockedTo(Entity);

#[derive(Component, Reflect)]
#[reflect(Component)]
#[relationship_target(relationship = DockedTo)]
struct DockedBy(Vec<Entity>);

/// A World with a small named hierarchy:
///
/// ```text
/// Floor ── West, Lane, Main      grunt, archer: Enemy (+ Health 10 on grunt)
/// Twin, Twin                     ship ─DockedTo→ planet
/// ```
struct Fixture {
    /// Holds the type registry as `AppTypeRegistry`, as an `App` would.
    world: World,
    floor: Entity,
    floor_children: [Entity; 3],
    grunt: Entity,
    archer: Entity,
    ship: Entity,
    planet: Entity,
}

impl Fixture {
    fn new() -> Self {
        let registry = AppTypeRegistry::default();
        let mut types = registry.write();
        types.register::<Name>();
        types.register::<Disabled>();
        types.register::<Children>();
        types.register::<ChildOf>();
        types.register::<Health>();
        types.register::<Enemy>();
        types.register::<Dead>();
        types.register::<Unspawned>();
        types.register::<DockedTo>();
        types.register::<DockedBy>();
        drop(types);

        let mut world = World::new();
        world.insert_resource(registry);
        let floor = world.spawn(Name::new("Floor")).id();
        let floor_children = ["West", "Lane", "Main"]
            .map(|name| world.spawn((Name::new(name), ChildOf(floor))).id());
        let grunt = world.spawn((Name::new("grunt"), Enemy, Health(10))).id();
        let archer = world.spawn((Name::new("archer"), Enemy, Dead)).id();
        world.spawn(Name::new("Twin"));
        world.spawn(Name::new("Twin"));
        let planet = world.spawn(Name::new("planet")).id();
        let ship = world.spawn((Name::new("ship"), DockedTo(planet))).id();
        Self {
            world,
            floor,
            floor_children,
            grunt,
            archer,
            ship,
            planet,
        }
    }

    /// Runs `query`, logging the result. See it with `--no-capture`.
    fn try_run(&mut self, query: &str) -> Result<QueryResult, ExecError> {
        let plan = QueryPlan::new(query, &self.world.resource::<AppTypeRegistry>().read())
            .unwrap_or_else(|error| panic!("{query:?}: {error}"));
        let result = plan.execute(&mut self.world);
        match &result {
            Ok(result) => println!("> {query}\n{result}"),
            Err(error) => println!("> {query}\nerror: {error}\n"),
        }
        result
    }

    fn run(&mut self, query: &str) -> QueryResult {
        self.try_run(query)
            .unwrap_or_else(|error| panic!("{query:?}: {error}"))
    }

    /// The result's source entities, in result order.
    fn sources(&mut self, query: &str) -> Vec<Entity> {
        self.run(query).rows.iter().map(|row| row.source).collect()
    }

    /// The result's source entities, sorted: scan order is unspecified.
    fn matched(&mut self, query: &str) -> Vec<Entity> {
        let mut entities = self.sources(query);
        entities.sort();
        entities
    }
}

fn sorted<const N: usize>(mut entities: [Entity; N]) -> Vec<Entity> {
    entities.sort();
    entities.to_vec()
}

fn health(value: &RowValue) -> Option<&Health> {
    match value {
        RowValue::Component(component) => component.downcast_ref(),
        _ => None,
    }
}

#[test]
fn required_fetches_skip_and_optional_fetches_keep() {
    let mut fixture = Fixture::new();
    let (grunt, archer) = (fixture.grunt, fixture.archer);
    let result = fixture.run("Health");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].source, grunt);
    assert_eq!(health(&result.rows[0].value), Some(&Health(10)));

    let result = fixture.run("(@ ?Health)[Enemy]");
    let absent = |entity| {
        let row = result.rows.iter().find(|row| row.source == entity).unwrap();
        let RowValue::Tuple(values) = &row.value else {
            panic!("expected a tuple");
        };
        matches!(values[1], RowValue::Absent)
    };
    assert_eq!(result.rows.len(), 2);
    assert!(!absent(grunt));
    assert!(absent(archer));
}

#[test]
fn filters_combine_and_negate() {
    let mut fixture = Fixture::new();
    let (grunt, archer) = (fixture.grunt, fixture.archer);
    assert_eq!(fixture.matched("@[Enemy !Dead]"), [grunt]);
    assert_eq!(
        fixture.matched("@[Enemy Dead or Health]"),
        sorted([grunt, archer])
    );
    assert_eq!(fixture.matched("@[Enemy !(Dead Enemy)]"), [grunt]);
    assert_eq!(fixture.matched("@[Enemy !#grunt]"), [archer]);
}

#[test]
fn names_select_every_matching_entity() {
    let mut fixture = Fixture::new();
    assert_eq!(fixture.sources("#Twin").len(), 2);
    assert_eq!(fixture.sources("#Nobody"), []);
}

#[test]
fn takes_follow_relationships_in_order() {
    let mut fixture = Fixture::new();
    let (floor, [west, lane, main]) = (fixture.floor, fixture.floor_children);
    assert_eq!(fixture.sources("#Floor | Children[..]"), [west, lane, main]);
    assert_eq!(fixture.sources("#Floor | Children[1]"), [lane]);
    assert_eq!(fixture.sources("#Floor | Children[1..]"), [lane, main]);
    assert_eq!(fixture.sources("#Floor | Children[..=1]"), [west, lane]);
    assert_eq!(fixture.sources("#Floor | Children[5]"), []);
    assert_eq!(fixture.sources("#Floor | Children[2..1]"), []);
    assert_eq!(fixture.sources("#Floor | Children[..][#Main]"), [main]);
    assert_eq!(fixture.sources("#West | ChildOf[0]"), [floor]);
}

#[test]
fn takes_are_per_row() {
    let mut fixture = Fixture::new();
    let second_parent = fixture.world.spawn(Name::new("Second")).id();
    let only_child = fixture.world.spawn(ChildOf(second_parent)).id();
    let first_child = fixture.floor_children[0];
    let firsts = fixture.matched("Children[0]");
    assert_eq!(firsts, sorted([first_child, only_child]));
}

#[test]
fn custom_relationships_follow_both_ways() {
    let mut fixture = Fixture::new();
    let (ship, planet) = (fixture.ship, fixture.planet);
    assert_eq!(fixture.sources("#ship | DockedTo[0]"), [planet]);
    assert_eq!(fixture.sources("#planet | DockedBy[..]"), [ship]);
}

#[test]
fn stages_connect_through_source_entities() {
    let mut fixture = Fixture::new();
    let grunt = fixture.grunt;
    let result = fixture.run("#grunt | Health | Name");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].source, grunt);
    let floor = fixture.floor;
    assert_eq!(
        fixture.sources("#Floor | Children[..] | ChildOf[0]"),
        [floor; 3]
    );
}

#[test]
fn literals_select_and_reject_entities() {
    let mut fixture = Fixture::new();
    let (grunt, archer) = (fixture.grunt, fixture.archer);
    assert_eq!(
        fixture.sources(&format!("@({grunt} {archer}) | Health")),
        [grunt]
    );

    fixture.world.entity_mut(archer).insert(Disabled);
    assert!(matches!(
        fixture.try_run(&format!("@{archer}")),
        Err(ExecError::FilteredEntity { .. })
    ));
    assert_eq!(fixture.matched("@[Enemy]"), [grunt]);

    fixture.world.despawn(grunt);
    assert!(matches!(
        fixture.try_run(&format!("@{grunt}")),
        Err(ExecError::StaleEntity { .. })
    ));
}

#[test]
fn uninitialized_components_are_absent() {
    let mut fixture = Fixture::new();
    assert!(fixture.run("Unspawned").rows.is_empty());
    assert!(fixture.run("@[Unspawned]").rows.is_empty());
    assert!(fixture.run("Unspawned[..]").rows.is_empty());
    let named = fixture.run("Name").rows.len();
    assert_eq!(fixture.run("@[Name !Unspawned]").rows.len(), named);
    let result = fixture.run("(Name ?Unspawned)");
    assert_eq!(result.rows.len(), named);
    assert!(result.rows.iter().all(|row| {
        matches!(&row.value, RowValue::Tuple(values) if matches!(values[1], RowValue::Absent))
    }));
}

#[test]
fn taking_a_non_relationship_is_an_error() {
    let mut fixture = Fixture::new();
    assert!(matches!(
        fixture.try_run("Health[..]"),
        Err(ExecError::NotARelationship { .. })
    ));
}

#[test]
fn displays_tuples_as_a_table() {
    let mut fixture = Fixture::new();
    let grunt = fixture.grunt;
    let table = fixture.run("(@ Health)").to_string();
    let lines: Vec<&str> = table.lines().collect();
    assert_eq!(lines.len(), 2, "{table}");
    assert!(
        lines[0].starts_with('@') && lines[0].ends_with("Health"),
        "{table}"
    );
    assert!(lines[1].starts_with(&grunt.to_string()), "{table}");
}

#[test]
fn selects_match_bevy_queries_on_default_filters() {
    let mut fixture = Fixture::new();
    let archer = fixture.archer;
    fixture.world.entity_mut(archer).insert(Disabled);
    // Mentioning `Disabled` lifts Bevy's default filter, exactly as
    // `Query<Entity, With<Disabled>>` does.
    let mut bevy: Vec<Entity> = fixture
        .world
        .query_filtered::<Entity, With<Disabled>>()
        .iter(&fixture.world)
        .collect();
    bevy.sort();
    assert_eq!(bevy, [archer]);
    assert_eq!(fixture.matched("@[Disabled]"), bevy);
    assert_eq!(fixture.matched("Name[Disabled Enemy]"), bevy);
    assert_eq!(fixture.matched("Name[Disabled or Dead]"), bevy);
    // Without mentioning it, disabled entities stay hidden.
    assert_eq!(fixture.matched("@[Dead]"), []);
}

#[test]
fn takes_skip_despawned_targets() {
    let mut fixture = Fixture::new();
    let ship = fixture.ship;
    let gone = fixture.world.spawn_empty().id();
    fixture.world.despawn(gone);
    // Bevy accepts relationship targets inserted with dead entries.
    fixture
        .world
        .spawn((Name::new("haunted"), DockedBy(vec![gone, ship])));
    assert_eq!(fixture.sources("#haunted | DockedBy[..][Name]"), [ship]);
    assert_eq!(fixture.sources("#haunted | DockedBy[0]"), [ship]);
}
