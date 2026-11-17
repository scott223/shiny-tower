use crate::room::{Room, RoomId, RoomType};

#[derive(Debug, Default)]
/// Building layout, including floors and elevator shafts.
pub struct Tower {
    pub floors: Vec<Floor>,
    pub elevator_shafts: Vec<ElevatorShaft>,
    next_room_id: u64,
}

#[derive(Debug, Default)]
/// A single tower level and the rooms placed on it.
pub struct Floor {
    pub number: u32,
    pub rooms: Vec<Room>,
}

#[derive(Debug, Default)]
/// An elevator shaft and the floors it connects.
pub struct ElevatorShaft {
    pub floors: Vec<u32>,
}

#[derive(Debug, PartialEq, Eq)]
/// Errors that prevent a room from being added to the tower.
pub enum TowerError {
    FloorNotFound(u32),
    RoomIdExhausted,
}

impl Tower {
    /// Creates a room at the given floor and horizontal tile position.
    ///
    /// Fails if the floor does not exist or the room ID counter is exhausted.
    pub fn build_room(
        &mut self,
        floor_number: u32,
        x: u32,
        room_type: RoomType,
    ) -> Result<Room, TowerError> {
        // Validate the target before allocating an ID so failed builds don't consume one.
        let floor = self
            .floors
            .iter_mut()
            .find(|floor| floor.number == floor_number)
            .ok_or(TowerError::FloorNotFound(floor_number))?;
        let next_room_id = self
            .next_room_id
            .checked_add(1)
            .ok_or(TowerError::RoomIdExhausted)?;
        let room = Room::new(RoomId(self.next_room_id), x, room_type);

        self.next_room_id = next_room_id;
        floor.rooms.push(room.clone());
        Ok(room)
    }

    /// Removes and returns a room if its ID exists on any floor.
    pub fn remove_room(&mut self, room_id: RoomId) -> Option<Room> {
        self.floors.iter_mut().find_map(|floor| {
            floor
                .rooms
                .iter()
                .position(|room| room.id == room_id)
                .map(|index| floor.rooms.remove(index))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// A room added to a floor can later be removed by its ID.
    fn built_room_can_be_removed_by_its_id() {
        let mut tower = Tower {
            floors: vec![Floor {
                number: 1,
                rooms: Vec::new(),
            }],
            ..Default::default()
        };

        let room = tower
            .build_room(1, 0, RoomType::Condo)
            .expect("room should build on an existing floor");
        assert_eq!(tower.floors[0].rooms.len(), 1);

        let removed_room = tower
            .remove_room(room.id)
            .expect("built room should be found by ID");
        assert_eq!(removed_room.id, room.id);
        assert!(tower.floors[0].rooms.is_empty());
    }
}
