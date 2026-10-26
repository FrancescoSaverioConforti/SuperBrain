#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome<T> {
    Reversible(T),
    Lossy { migrated: T, lost_fields: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationError {
    UnsupportedVersion { from: u32, to: u32 },
}

pub trait Migration {
    type Input;
    type Output;
    fn migrate(&self, input: Self::Input) -> Result<MigrationOutcome<Self::Output>, MigrationError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct FixtureV1 { id: String, label: String }
    #[derive(Clone, Debug, PartialEq)]
    struct FixtureV2 { id: String, label: String, note: Option<String> }

    struct ExampleMigration;
    impl Migration for ExampleMigration {
        type Input = FixtureV1;
        type Output = FixtureV2;
        fn migrate(&self, input: FixtureV1) -> Result<MigrationOutcome<FixtureV2>, MigrationError> {
            Ok(MigrationOutcome::Reversible(FixtureV2 { id: input.id, label: input.label, note: None }))
        }
    }

    #[test]
    fn example_pair_v1_migrates_to_expected_v2() {
        let v1 = FixtureV1 { id: "x".into(), label: "hello".into() };
        let expected_v2 = FixtureV2 { id: "x".into(), label: "hello".into(), note: None };
        let outcome = ExampleMigration.migrate(v1).unwrap();
        let migrated = match outcome { MigrationOutcome::Reversible(v) => v, _ => panic!("expected reversible") };
        assert_eq!(migrated, expected_v2);
    }

    #[test]
    fn migration_applied_twice_to_the_same_input_yields_the_same_output() {
        let v1 = FixtureV1 { id: "x".into(), label: "hello".into() };
        let once = ExampleMigration.migrate(v1.clone()).unwrap();
        let twice = ExampleMigration.migrate(v1).unwrap();
        let unwrap_reversible = |o: MigrationOutcome<FixtureV2>| match o {
            MigrationOutcome::Reversible(v) => v, _ => panic!("expected reversible"),
        };
        assert_eq!(unwrap_reversible(once), unwrap_reversible(twice));
    }

    #[test]
    fn dropping_the_added_field_reverses_v2_back_to_v1() {
        let v1 = FixtureV1 { id: "x".into(), label: "hello".into() };
        let outcome = ExampleMigration.migrate(v1.clone()).unwrap();
        let v2 = match outcome { MigrationOutcome::Reversible(v) => v, _ => panic!("expected reversible") };
        let reversed = FixtureV1 { id: v2.id, label: v2.label }; // dropping `note` is the declared-reversible path
        assert_eq!(reversed, v1);
    }
}
