#[derive(Debug, Default)]
/// People tracked by the simulation.
pub struct People {
    /// All people currently represented in the tower.
    pub members: Vec<Person>,
}

#[derive(Debug, Default)]
/// One person associated with the tower.
pub struct Person {
    /// Stable identifier for this person.
    pub id: u64,
    /// Display name.
    pub name: String,
    /// Floor the person is currently on.
    pub floor: i32,
}
