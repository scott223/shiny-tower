#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
/// A kind of room that can be built in the tower.
pub enum RoomType {
    #[default]
    Condo,
    PartyHall,
    Lobby,
    Office,
    FastFood,
    RetailShop,
    Restaurant,
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
    /// Number of people the room can accommodate.
    pub population: u32,
    /// One-time profit earned when the room is built.
    pub profit_instant: u64,
    /// Recurring profit earned each simulation day at 09:00.
    pub profit_daily: u64,
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
                population: 3,
                profit_instant: 200,
                profit_daily: 0,
                profit_quarterly: 0,
            },
            Self::PartyHall => RoomProperties {
                width: 24,
                height: 2,
                cost: 500,
                population: 0,
                profit_instant: 0,
                profit_daily: 0,
                profit_quarterly: 0,
            },
            Self::Lobby => RoomProperties {
                width: 1,
                height: 1,
                cost: 150,
                population: 0,
                profit_instant: 0,
                profit_daily: 0,
                profit_quarterly: 0,
            },
            Self::Office => RoomProperties {
                width: 9,
                height: 1,
                cost: 250,
                population: 6,
                profit_instant: 0,
                profit_daily: 0,
                profit_quarterly: 50,
            },
            Self::FastFood => RoomProperties {
                width: 16,
                height: 1,
                cost: 100_000,
                population: 0,
                profit_instant: 0,
                profit_daily: 2_000,
                profit_quarterly: 0,
            },
            Self::RetailShop => RoomProperties {
                width: 12,
                height: 1,
                cost: 100_000,
                population: 0,
                profit_instant: 0,
                profit_daily: 0,
                profit_quarterly: 0,
            },
            Self::Restaurant => RoomProperties {
                width: 24,
                height: 1,
                cost: 200_000,
                population: 0,
                profit_instant: 0,
                profit_daily: 4_000,
                profit_quarterly: 0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_commercial_rooms_have_the_requested_sizes_and_costs() {
        assert_eq!(
            RoomType::FastFood.properties(),
            RoomProperties {
                width: 16,
                height: 1,
                cost: 100_000,
                population: 0,
                profit_instant: 0,
                profit_daily: 2_000,
                profit_quarterly: 0,
            }
        );
        assert_eq!(
            RoomType::RetailShop.properties(),
            RoomProperties {
                width: 12,
                height: 1,
                cost: 100_000,
                population: 0,
                profit_instant: 0,
                profit_daily: 0,
                profit_quarterly: 0,
            }
        );
        assert_eq!(
            RoomType::Restaurant.properties(),
            RoomProperties {
                width: 24,
                height: 1,
                cost: 200_000,
                population: 0,
                profit_instant: 0,
                profit_daily: 4_000,
                profit_quarterly: 0,
            }
        );
    }

    #[test]
    fn condo_and_office_have_requested_population_capacity() {
        assert_eq!(RoomType::Condo.properties().population, 3);
        assert_eq!(RoomType::Office.properties().population, 6);
    }
}

#[derive(Clone, Debug)]
/// A room instance placed on a floor.
pub struct Room {
    /// Leftmost column occupied by the room.
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
