slotmap::new_key_type! {
    /// Stable identifier assigned to an elevator shaft.
    pub struct ElevatorId;
}

#[derive(Debug)]
/// An elevator shaft and the floors it connects.
pub struct ElevatorShaft {
    /// Column occupied by the shaft.
    pub column: u32,
    /// First connected floor, inclusive.
    pub start_floor: i32,
    /// Last connected floor, inclusive.
    pub end_floor: i32,
}
