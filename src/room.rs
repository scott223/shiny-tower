#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// A kind of room that can be built in the tower.
pub enum RoomType {
    #[default]
    Condo,
    PartyHall,
    Lobby,
    Office,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Stable identifier assigned to a room when it is built.
pub struct RoomId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Fixed dimensions and construction cost for a room type.
pub struct RoomProperties {
    pub width: u32,
    pub height: u32,
    pub cost: u64,
}

impl RoomType {
    /// Returns the configured dimensions and cost for this type.
    pub const fn properties(self) -> RoomProperties {
        match self {
            Self::Condo => RoomProperties {
                width: 16,
                height: 1,
                cost: 100,
            },
            Self::PartyHall => RoomProperties {
                width: 24,
                height: 2,
                cost: 500,
            },
            Self::Lobby => RoomProperties {
                width: 1,
                height: 1,
                cost: 150,
            },
            Self::Office => RoomProperties {
                width: 9,
                height: 1,
                cost: 250,
            },
        }
    }
}

#[derive(Clone, Debug)]
/// A room instance placed on a floor.
pub struct Room {
    /// Unique room identifier.
    pub id: RoomId,
    /// Horizontal position in tiles.
    pub x: u32,
    /// Kind of room and source of its properties.
    pub room_type: RoomType,
    /// Dimensions and cost copied from the room type.
    pub properties: RoomProperties,
}

impl Room {
    /// Creates a room and initializes its properties from its type.
    pub fn new(id: RoomId, x: u32, room_type: RoomType) -> Self {
        Self {
            id,
            x,
            room_type,
            properties: room_type.properties(),
        }
    }
}
