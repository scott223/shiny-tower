use crate::elevator::{ElevatorId, ElevatorShaft};
use crate::room::{Room, RoomId, RoomType};
use slotmap::SlotMap;

pub const FLOORS_ABOVE_GROUND: i32 = 100;
pub const FLOORS_BELOW_GROUND: i32 = 10;
pub const MAX_FLOOR_WIDTH_COLUMNS: u32 = 400;
pub const FLOOR_SEGMENT_COST: u64 = 500;

#[derive(Debug)]
/// Building layout, including floors and elevator shafts.
pub struct Tower {
    pub floors: Vec<Floor>,
    pub rooms: SlotMap<RoomId, Room>,
    pub elevator_shafts: SlotMap<ElevatorId, ElevatorShaft>,
}

struct RoomBuildPlan {
    floor_index: usize,
    missing_segments: Vec<u32>,
}

#[derive(Debug, Default)]
/// A single floor and the rooms placed on it.
pub struct Floor {
    pub number: i32,
    pub floor_segments: Vec<u32>,
    pub rooms: Vec<RoomId>,
}

#[derive(Debug, PartialEq, Eq)]
/// Errors returned by tower operations.
pub enum TowerError {
    FloorNotFound(i32),
    ColumnOutOfRange(u32),
    RoomExceedsFloorWidth { x: u32, width: u32 },
    FloorSegmentBelowGround { floor: i32, column: u32 },
    FloorSegmentAlreadyExists { floor: i32, column: u32 },
    FloorSegmentUnsupported { floor: i32, column: u32 },
    FloorSegmentOverlapsElevator { floor: i32, column: u32 },
    RoomOverlap(RoomId),
    ElevatorOverlap { column: u32 },
    InvalidElevatorFloorRange { start_floor: i32, end_floor: i32 },
    ElevatorOverlapsRoom { room_id: RoomId, floor: i32 },
    ElevatorNotFound(ElevatorId),
    RoomNotFound(RoomId),
}

impl Default for Tower {
    fn default() -> Self {
        Self {
            floors: (-FLOORS_BELOW_GROUND..FLOORS_ABOVE_GROUND)
                .map(|number| Floor {
                    number,
                    floor_segments: Vec::new(),
                    rooms: Vec::new(),
                })
                .collect(),
            rooms: SlotMap::with_key(),
            elevator_shafts: SlotMap::with_key(),
        }
    }
}

impl Tower {
    fn column_has_support_below(&self, floor_number: i32, column: u32) -> Result<bool, TowerError> {
        let below_floor_number = floor_number
            .checked_sub(1)
            .ok_or(TowerError::FloorNotFound(floor_number))?;
        let below_floor = self
            .floors
            .iter()
            .find(|floor| floor.number == below_floor_number)
            .ok_or(TowerError::FloorNotFound(below_floor_number))?;
        let supported_by_segment = below_floor.floor_segments.contains(&column);
        let supported_by_room = below_floor
            .rooms
            .iter()
            .filter_map(|room_id| self.rooms.get(*room_id))
            .any(|room| {
                let room_left = u64::from(room.x);
                let room_right = room_left + u64::from(room.room_type.properties().width);
                room_left <= u64::from(column) && u64::from(column) < room_right
            });
        Ok(supported_by_segment || supported_by_room)
    }

    /// Builds one floor segment at the given column.
    ///
    /// Floor 0 is the foundation. Higher segments require a segment or room directly below.
    pub fn build_floor_segment(
        &mut self,
        floor_number: i32,
        column: u32,
    ) -> Result<(), TowerError> {
        let floor_index = self
            .floors
            .iter()
            .position(|floor| floor.number == floor_number)
            .ok_or(TowerError::FloorNotFound(floor_number))?;
        if column >= MAX_FLOOR_WIDTH_COLUMNS {
            return Err(TowerError::ColumnOutOfRange(column));
        }
        if floor_number < 0 {
            return Err(TowerError::FloorSegmentBelowGround {
                floor: floor_number,
                column,
            });
        }
        if self.floors[floor_index].floor_segments.contains(&column) {
            return Err(TowerError::FloorSegmentAlreadyExists {
                floor: floor_number,
                column,
            });
        }
        if self.elevator_shafts.values().any(|elevator| {
            elevator.start_floor <= floor_number
                && floor_number <= elevator.end_floor
                && elevator.column == column
        }) {
            return Err(TowerError::FloorSegmentOverlapsElevator {
                floor: floor_number,
                column,
            });
        }

        if floor_number > 0 {
            if !self.column_has_support_below(floor_number, column)? {
                return Err(TowerError::FloorSegmentUnsupported {
                    floor: floor_number,
                    column,
                });
            }
        }

        self.floors[floor_index].floor_segments.push(column);
        Ok(())
    }

    fn plan_room_build(
        &self,
        floor_number: i32,
        x: u32,
        room_type: RoomType,
    ) -> Result<RoomBuildPlan, TowerError> {
        // Resolve the target floor before validating placement.
        let floor_index = self
            .floors
            .iter()
            .position(|floor| floor.number == floor_number)
            .ok_or(TowerError::FloorNotFound(floor_number))?;

        let width = room_type.properties().width;
        // Use a right-exclusive edge to match the overlap check below.
        let right_edge = i64::from(x) + i64::from(width);
        if right_edge > i64::from(MAX_FLOOR_WIDTH_COLUMNS) {
            return Err(TowerError::RoomExceedsFloorWidth { x, width });
        }

        // Elevator shafts occupy their column on every floor in the inclusive range.
        if let Some(elevator) = self.elevator_shafts.values().find(|elevator| {
            elevator.start_floor <= floor_number
                && floor_number <= elevator.end_floor
                && i64::from(x) <= i64::from(elevator.column)
                && i64::from(elevator.column) < right_edge
        }) {
            return Err(TowerError::ElevatorOverlap {
                column: elevator.column,
            });
        }

        let mut missing_segments = Vec::new();
        for column in x..x + width {
            if !self.floors[floor_index].floor_segments.contains(&column) {
                if floor_number < 0 {
                    return Err(TowerError::FloorSegmentBelowGround {
                        floor: floor_number,
                        column,
                    });
                }
                if floor_number > 0 && !self.column_has_support_below(floor_number, column)? {
                    return Err(TowerError::FloorSegmentUnsupported {
                        floor: floor_number,
                        column,
                    });
                }
                missing_segments.push(column);
            }
        }

        // Rooms occupy [x, right_edge); touching edges are allowed, shared columns are not.
        if let Some(overlapping_room) =
            self.floors[floor_index]
                .rooms
                .iter()
                .copied()
                .find_map(|room_id| {
                    // Skip stale floor keys rather than indexing a missing slotmap entry.
                    let room = self.rooms.get(room_id)?;
                    let existing_left_edge = i64::from(room.x);
                    let existing_right_edge =
                        existing_left_edge + i64::from(room.room_type.properties().width);
                    (i64::from(x) < existing_right_edge && existing_left_edge < right_edge)
                        .then_some(room_id)
                })
        {
            return Err(TowerError::RoomOverlap(overlapping_room));
        }

        Ok(RoomBuildPlan {
            floor_index,
            missing_segments,
        })
    }

    /// Returns the room's cost plus the cost of the floor segments it would add.
    pub(crate) fn room_build_cost(
        &self,
        floor_number: i32,
        x: u32,
        room_type: RoomType,
    ) -> Result<u64, TowerError> {
        let plan = self.plan_room_build(floor_number, x, room_type)?;
        let missing_segment_cost = plan
            .missing_segments
            .iter()
            .map(|_| FLOOR_SEGMENT_COST)
            .sum::<u64>();
        Ok(room_type.properties().cost + missing_segment_cost)
    }

    /// Creates a room at the given floor and column.
    ///
    /// Missing floor segments are added if they have support below. The build fails if any
    /// required segment is unsupported, the room exceeds the floor width, overlaps another room,
    /// or overlaps an elevator shaft.
    pub fn build_room(
        &mut self,
        floor_number: i32,
        x: u32,
        room_type: RoomType,
    ) -> Result<RoomId, TowerError> {
        let plan = self.plan_room_build(floor_number, x, room_type)?;

        // Store the room once and keep its generated key in the floor's room list.
        self.floors[plan.floor_index]
            .floor_segments
            .extend(plan.missing_segments);
        let room_id = self.rooms.insert(Room::new(x, room_type));
        self.floors[plan.floor_index].rooms.push(room_id);
        Ok(room_id)
    }

    /// Builds an elevator shaft spanning the inclusive floor range.
    ///
    /// Fails if the range is reversed, either endpoint does not exist, the column is outside the
    /// tower, or an existing room occupies that column on a spanned floor.
    pub fn build_elevator_shaft(
        &mut self,
        column: u32,
        start_floor: i32,
        end_floor: i32,
    ) -> Result<ElevatorId, TowerError> {
        if start_floor > end_floor {
            return Err(TowerError::InvalidElevatorFloorRange {
                start_floor,
                end_floor,
            });
        }
        if column >= MAX_FLOOR_WIDTH_COLUMNS {
            return Err(TowerError::ColumnOutOfRange(column));
        }
        for floor_number in [start_floor, end_floor] {
            if !self.floors.iter().any(|floor| floor.number == floor_number) {
                return Err(TowerError::FloorNotFound(floor_number));
            }
        }

        for floor in self
            .floors
            .iter()
            .filter(|floor| start_floor <= floor.number && floor.number <= end_floor)
        {
            for room_id in &floor.rooms {
                let Some(room) = self.rooms.get(*room_id) else {
                    continue;
                };
                let room_left = i64::from(room.x);
                let room_right = room_left + i64::from(room.room_type.properties().width);
                if room_left <= i64::from(column) && i64::from(column) < room_right {
                    return Err(TowerError::ElevatorOverlapsRoom {
                        room_id: *room_id,
                        floor: floor.number,
                    });
                }
            }
            if floor.floor_segments.contains(&column) {
                return Err(TowerError::FloorSegmentOverlapsElevator {
                    floor: floor.number,
                    column,
                });
            }
        }

        Ok(self.elevator_shafts.insert(ElevatorShaft {
            column,
            start_floor,
            end_floor,
        }))
    }

    /// Removes and returns an elevator shaft, or errors if its ID is not in the slotmap.
    pub fn remove_elevator_shaft(
        &mut self,
        elevator_id: ElevatorId,
    ) -> Result<ElevatorShaft, TowerError> {
        self.elevator_shafts
            .remove(elevator_id)
            .ok_or(TowerError::ElevatorNotFound(elevator_id))
    }

    /// Removes and returns a room, or errors if its ID is not in the slotmap.
    pub fn remove_room(&mut self, room_id: RoomId) -> Result<Room, TowerError> {
        let room = self
            .rooms
            .remove(room_id)
            .ok_or(TowerError::RoomNotFound(room_id))?;
        for floor in &mut self.floors {
            floor.rooms.retain(|id| *id != room_id);
        }
        Ok(room)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_supported_segments(
        tower: &mut Tower,
        floor_number: i32,
        columns: std::ops::Range<u32>,
    ) {
        for column in columns {
            for floor in 0..=floor_number {
                tower
                    .build_floor_segment(floor, column)
                    .expect("floor segment should have support");
            }
        }
    }

    #[test]
    fn default_tower_has_floors_above_and_below_ground() {
        let tower = Tower::default();

        assert_eq!(
            tower.floors.len(),
            (FLOORS_ABOVE_GROUND + FLOORS_BELOW_GROUND) as usize
        );
        assert_eq!(
            tower.floors.first().map(|floor| floor.number),
            Some(-FLOORS_BELOW_GROUND)
        );
        assert_eq!(
            tower.floors.last().map(|floor| floor.number),
            Some(FLOORS_ABOVE_GROUND - 1)
        );
        assert!(tower.floors.iter().any(|floor| floor.number == 0));
        assert!(tower.floors.iter().any(|floor| floor.number == -1));
    }

    #[test]
    fn floor_segments_require_support_and_can_be_built_upward() {
        let mut tower = Tower::default();

        assert!(matches!(
            tower.build_floor_segment(1, 5),
            Err(TowerError::FloorSegmentUnsupported {
                floor: 1,
                column: 5
            })
        ));
        tower
            .build_floor_segment(0, 5)
            .expect("ground floor is the foundation");
        tower
            .build_floor_segment(1, 5)
            .expect("ground segment should support the next floor");
        tower
            .build_floor_segment(2, 5)
            .expect("floor segment should support the floor above it");

        assert!(matches!(
            tower.build_floor_segment(-1, 5),
            Err(TowerError::FloorSegmentBelowGround {
                floor: -1,
                column: 5
            })
        ));
    }

    #[test]
    fn floor_segment_can_be_supported_by_a_room_below() {
        let mut tower = Tower::default();
        tower
            .build_floor_segment(0, 8)
            .expect("ground floor is the foundation");
        tower
            .build_room(0, 8, RoomType::Lobby)
            .expect("room should build on its floor segment");
        let ground_floor = tower
            .floors
            .iter_mut()
            .find(|floor| floor.number == 0)
            .expect("ground floor should exist");
        // Remove the segment to verify the room alone can support the floor above.
        ground_floor.floor_segments.clear();

        tower
            .build_floor_segment(1, 8)
            .expect("room in the column below should support the segment");
    }

    #[test]
    fn room_build_automatically_adds_ground_and_supported_floor_segments() {
        let mut tower = Tower::default();
        assert!(tower.build_room(0, 0, RoomType::Condo).is_ok());

        let ground_floor = tower
            .floors
            .iter()
            .find(|floor| floor.number == 0)
            .expect("ground floor should exist");
        assert_eq!(ground_floor.floor_segments, (0..16).collect::<Vec<_>>());

        assert!(tower.build_room(1, 0, RoomType::Condo).is_ok());
        let first_floor = tower
            .floors
            .iter()
            .find(|floor| floor.number == 1)
            .expect("first floor should exist");
        assert_eq!(first_floor.floor_segments, (0..16).collect::<Vec<_>>());
    }

    #[test]
    fn room_can_auto_build_floor_segments_supported_by_a_room_below() {
        let mut tower = Tower::default();
        tower
            .build_floor_segment(0, 8)
            .expect("ground floor is the foundation");
        tower
            .build_room(0, 8, RoomType::Lobby)
            .expect("room should build on its floor segment");
        let ground_floor = tower
            .floors
            .iter_mut()
            .find(|floor| floor.number == 0)
            .expect("ground floor should exist");
        // Remove the segment to isolate the room as the support source.
        ground_floor.floor_segments.clear();

        tower
            .build_room(1, 8, RoomType::Lobby)
            .expect("room below should support the automatically added segment");
        assert!(
            tower
                .floors
                .iter()
                .find(|floor| floor.number == 1)
                .expect("first floor should exist")
                .floor_segments
                .contains(&8)
        );
    }

    #[test]
    fn unsupported_room_build_does_not_add_partial_floor_segments() {
        let mut tower = Tower::default();
        build_supported_segments(&mut tower, 0, 0..15);

        assert!(matches!(
            tower.build_room(1, 0, RoomType::Condo),
            Err(TowerError::FloorSegmentUnsupported {
                floor: 1,
                column: 15
            })
        ));
        assert!(
            tower
                .floors
                .iter()
                .find(|floor| floor.number == 1)
                .expect("first floor should exist")
                .floor_segments
                .is_empty()
        );
    }

    #[test]
    fn room_must_fit_within_the_400_column_floor_width() {
        let mut tower = Tower::default();
        let condo_width = RoomType::Condo.properties().width;
        build_supported_segments(
            &mut tower,
            0,
            MAX_FLOOR_WIDTH_COLUMNS - condo_width..MAX_FLOOR_WIDTH_COLUMNS,
        );

        assert!(
            tower
                .build_room(0, MAX_FLOOR_WIDTH_COLUMNS - condo_width, RoomType::Condo,)
                .is_ok()
        );
        assert!(matches!(
            tower.build_room(
                0,
                MAX_FLOOR_WIDTH_COLUMNS - condo_width + 1,
                RoomType::Condo,
            ),
            Err(TowerError::RoomExceedsFloorWidth { .. })
        ));
    }

    #[test]
    fn room_build_is_rejected_if_its_columns_overlap_an_existing_room() {
        let mut tower = Tower::default();
        build_supported_segments(&mut tower, 0, 9..35);
        let existing_room_id = tower
            .build_room(0, 10, RoomType::Condo)
            .expect("first room should build");

        assert!(matches!(
            tower.build_room(0, 9, RoomType::Office),
            Err(TowerError::RoomOverlap(room_id)) if room_id == existing_room_id
        ));
        assert!(
            tower.build_room(0, 26, RoomType::Office).is_ok(),
            "rooms whose edges touch should not overlap"
        );
    }

    #[test]
    fn room_build_is_rejected_if_it_overlaps_an_elevator_on_that_floor() {
        let mut tower = Tower::default();
        let elevator_id = tower
            .build_elevator_shaft(12, -1, 1)
            .expect("elevator shaft should build across empty floors");

        assert!(matches!(
            tower.build_floor_segment(0, 12),
            Err(TowerError::FloorSegmentOverlapsElevator {
                floor: 0,
                column: 12
            })
        ));
        for floor in [-1, 0, 1] {
            assert!(matches!(
                tower.build_room(floor, 12, RoomType::Lobby),
                Err(TowerError::ElevatorOverlap { column: 12 })
            ));
        }
        build_supported_segments(&mut tower, 2, 13..22);
        assert!(tower.build_room(2, 13, RoomType::Office).is_ok());
        tower
            .remove_elevator_shaft(elevator_id)
            .expect("elevator shaft should be removable by its ID");
        tower
            .build_floor_segment(0, 12)
            .expect("ground floor segment should build after elevator removal");
        assert!(tower.build_room(0, 12, RoomType::Lobby).is_ok());
        assert!(matches!(
            tower.remove_elevator_shaft(elevator_id),
            Err(TowerError::ElevatorNotFound(missing_id)) if missing_id == elevator_id
        ));
    }

    #[test]
    fn elevator_build_is_rejected_if_a_room_occupies_its_column() {
        let mut tower = Tower::default();
        build_supported_segments(&mut tower, 0, 10..26);
        let room_id = tower
            .build_room(0, 10, RoomType::Condo)
            .expect("room should build");

        assert!(matches!(
            tower.build_elevator_shaft(12, -1, 1),
            Err(TowerError::ElevatorOverlapsRoom {
                room_id: overlapping_id,
                floor: 0
            }) if overlapping_id == room_id
        ));
    }

    #[test]
    /// A room added to a floor can later be removed by its ID.
    fn built_room_can_be_removed_by_its_id() {
        let mut tower = Tower::default();
        build_supported_segments(&mut tower, 1, 0..16);
        let floor_index = tower
            .floors
            .iter()
            .position(|floor| floor.number == 1)
            .expect("floor should exist");

        let room = tower
            .build_room(1, 0, RoomType::Condo)
            .expect("room should build on an existing floor");
        assert_eq!(tower.floors[floor_index].rooms.len(), 1);

        let removed_room = tower
            .remove_room(room)
            .expect("built room should be found by ID");
        assert_eq!(removed_room.x, 0);
        assert!(tower.floors[floor_index].rooms.is_empty());
        assert!(tower.rooms.is_empty());

        let replacement = tower
            .build_room(1, 0, RoomType::Condo)
            .expect("room should be buildable after removal");
        assert_ne!(replacement, room);
        assert!(tower.rooms.get(room).is_none());
        assert!(tower.rooms.get(replacement).is_some());
    }

    #[test]
    fn removing_unknown_room_returns_an_error() {
        let mut tower = Tower::default();
        build_supported_segments(&mut tower, 0, 0..16);
        let room_id = tower
            .build_room(0, 0, RoomType::Condo)
            .expect("room should build");
        tower
            .remove_room(room_id)
            .expect("first removal should succeed");

        assert!(matches!(
            tower.remove_room(room_id),
            Err(TowerError::RoomNotFound(missing_id)) if missing_id == room_id
        ));
    }
}
