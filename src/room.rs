#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// A kind of room that can be built in the tower.
pub enum RoomType {
    #[default]
    Condo,
    PartyHall,
    Lobby,
    Office,
}

slotmap::new_key_type! {
    /// Stable identifier assigned to a room when it is built.
    pub struct RoomId;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Fixed dimensions, cost, and profit values for a room type.
pub struct RoomProperties {
    /// Width in columns.
    pub width: u32,
    /// Height in floors.
    pub height: u32,
    pub cost: u64,
    /// One-time profit earned when the room is built.
    pub profit_instant: u64,
    /// Recurring profit earned each quarter.
    pub profit_quarterly: u64,
}

impl RoomType {
    /// Returns the configured properties for this type.
    pub const fn properties(self) -> RoomProperties {
        match self {
            Self::Condo => RoomProperties {
                width: 16,
                height: 1,
                cost: 100,
                profit_instant: 200,
                profit_quarterly: 0,
            },
            Self::PartyHall => RoomProperties {
                width: 24,
                height: 2,
                cost: 500,
                profit_instant: 0,
                profit_quarterly: 0,
            },
            Self::Lobby => RoomProperties {
                width: 1,
                height: 1,
                cost: 150,
                profit_instant: 0,
                profit_quarterly: 0,
            },
            Self::Office => RoomProperties {
                width: 9,
                height: 1,
                cost: 250,
                profit_instant: 0,
                profit_quarterly: 50,
            },
        }
    }
}

#[derive(Clone, Debug)]
/// A room instance placed on a floor.
pub struct Room {
    /// Column position on the floor.
    pub x: u32,
    /// Kind of room, which defines its properties.
    pub room_type: RoomType,
}

impl Room {
    /// Creates a room of the given type.
    pub fn new(x: u32, room_type: RoomType) -> Self {
        Self { x, room_type }
    }
}
