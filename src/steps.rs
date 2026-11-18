slotmap::new_key_type! {
    /// Stable identifier assigned to a set of steps.
    pub struct StepsId;
}

/// A set of steps spanning two floors.
#[derive(Clone, Debug)]
pub struct Steps {
    /// Leftmost column occupied by the steps.
    pub x: u32,
    /// Lower floor occupied by the steps.
    pub start_floor: i32,
}

impl Steps {
    /// Horizontal width of a set of steps in columns.
    pub const WIDTH: u32 = 8;
    /// Number of floors spanned by the steps.
    pub const HEIGHT: i32 = 2;
    /// Cost to build a set of steps.
    pub const COST: u64 = 5_000;

    /// Creates a set of steps at the given column and lower floor.
    pub fn new(x: u32, start_floor: i32) -> Self {
        Self { x, start_floor }
    }
}
