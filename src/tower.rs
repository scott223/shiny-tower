use crate::elevator::{ElevatorId, ElevatorShaft};
use crate::room::{Room, RoomId, RoomType};
use slotmap::SlotMap;

/// Number of floors from ground level upward, including ground.
pub const FLOORS_ABOVE_GROUND: i32 = 109;
/// Number of floors below ground level.
pub const FLOORS_BELOW_GROUND: i32 = 10;
/// Number of columns available on each floor.
pub const MAX_FLOOR_WIDTH_COLUMNS: u32 = 400;
/// Cost of building one floor segment.
pub const FLOOR_SEGMENT_COST: u64 = 500;

#[derive(Debug)]
/// Building layout with floors numbered from `-FLOORS_BELOW_GROUND` through
/// `FLOORS_ABOVE_GROUND - 1`, including rooms and elevator shafts.
pub struct Tower {
    /// Floors in ascending numeric order.
    pub floors: Vec<Floor>,
    /// Rooms indexed by their stable IDs.
    pub rooms: SlotMap<RoomId, Room>,
    /// Elevator shafts indexed by their stable IDs.
    pub elevator_shafts: SlotMap<ElevatorId, ElevatorShaft>,
}

/// Validated placement data used to price or commit a room build.
struct RoomBuildPlan {
    /// Index of the floor where the room will be placed.
    floor_index: usize,
    /// Floor columns that construction must add to support the room.
    missing_segments: Vec<u32>,
}

#[derive(Debug, Default)]
/// A single floor and the rooms placed on it.
pub struct Floor {
    /// Numeric floor level; zero is ground level.
    pub number: i32,
    /// Columns occupied by floor segments, including segments added for rooms.
    pub floor_segments: Vec<u32>,
    /// IDs of rooms placed on this floor.
    pub rooms: Vec<RoomId>,
}

#[derive(Debug, PartialEq, Eq)]
/// Errors returned by tower operations.
pub enum TowerError {
    /// The requested floor does not exist.
    FloorNotFound(i32),
    /// A column is outside the tower's horizontal bounds.
    ColumnOutOfRange(u32),
    /// A room footprint extends past the last column on its floor.
    RoomExceedsFloorWidth { x: u32, width: u32 },
    /// A required floor segment cannot be built below ground level.
    FloorSegmentBelowGround { floor: i32, column: u32 },
    /// A segment already occupies this floor and column.
    FloorSegmentAlreadyExists { floor: i32, column: u32 },
    /// No room or segment directly below supports this segment.
    FloorSegmentUnsupported { floor: i32, column: u32 },
    /// An elevator shaft already occupies this floor and column.
    FloorSegmentOverlapsElevator { floor: i32, column: u32 },
    /// Only a lobby may be placed on ground level; standalone segments are not allowed there.
    GroundFloorLobbyOnly,
    /// A lobby may be placed only on ground level or a positive multiple of 15.
    LobbyOnlyAllowedOnEveryFifteenthFloor { floor: i32 },
    /// The proposed room footprint intersects an existing room.
    RoomOverlap(RoomId),
    /// The elevator's start floor is above its end floor.
    InvalidElevatorFloorRange { start_floor: i32, end_floor: i32 },
    /// The requested elevator shaft does not exist.
    ElevatorNotFound(ElevatorId),
    /// The requested room does not exist.
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
        // Support must come from the immediately lower floor, not any floor further below.
        let below_floor_number = floor_number
            .checked_sub(1)
            .ok_or(TowerError::FloorNotFound(floor_number))?;
        let below_floor = self
            .floors
            .iter()
            .find(|floor| floor.number == below_floor_number)
            .ok_or(TowerError::FloorNotFound(below_floor_number))?;

        // A prebuilt segment supports exactly its own column.
        let supported_by_segment = below_floor.floor_segments.contains(&column);

        // Rooms support every column in their half-open horizontal footprint.
        let supported_by_room = below_floor
            .rooms
            .iter()
            .filter_map(|room_id| self.rooms.get(*room_id))
            .any(|room| {
                let room_left = u64::from(room.x);
                let room_right = room_left + u64::from(room.room_type.properties().width);
                room_left <= u64::from(column) && u64::from(column) < room_right
            });

        // Either kind of structure on the lower floor is sufficient support.
        Ok(supported_by_segment || supported_by_room)
    }

    /// Builds one floor segment at the given column.
    ///
    /// Segments cannot be placed below ground or independently at ground level; a ground-floor
    /// lobby may add its occupied support segment. Higher floors require support directly below.
    pub fn build_floor_segment(
        &mut self,
        floor_number: i32,
        column: u32,
    ) -> Result<(), TowerError> {
        // Find the target first so later checks and insertion operate on a known floor.
        let floor_index = self
            .floors
            .iter()
            .position(|floor| floor.number == floor_number)
            .ok_or(TowerError::FloorNotFound(floor_number))?;

        // Segments occupy a single column within the tower's fixed horizontal bounds.
        if column >= MAX_FLOOR_WIDTH_COLUMNS {
            return Err(TowerError::ColumnOutOfRange(column));
        }

        // The modeled foundation begins at ground level; below-ground construction is unsupported.
        if floor_number < 0 {
            return Err(TowerError::FloorSegmentBelowGround {
                floor: floor_number,
                column,
            });
        }

        // Ground level is reserved for lobbies; floor segments are not built there independently.
        if floor_number == 0 {
            return Err(TowerError::GroundFloorLobbyOnly);
        }

        // Do not create duplicate structure at the same floor and column.
        if self.floors[floor_index].floor_segments.contains(&column) {
            return Err(TowerError::FloorSegmentAlreadyExists {
                floor: floor_number,
                column,
            });
        }

        // Elevator shafts reserve their column throughout their inclusive floor range.
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

        // All permitted standalone segments are above ground and require direct support.
        if floor_number > 0 {
            if !self.column_has_support_below(floor_number, column)? {
                return Err(TowerError::FloorSegmentUnsupported {
                    floor: floor_number,
                    column,
                });
            }
        }

        // Commit only after all placement and support checks pass.
        self.floors[floor_index].floor_segments.push(column);
        Ok(())
    }

    /// Validates a proposed room placement without mutating the tower.
    ///
    /// Returns the target floor and the supported floor segments that construction would need
    /// to add. Callers can use this plan to determine cost before committing the build.
    fn plan_room_build(
        &self,
        floor_number: i32,
        x: u32,
        room_type: RoomType,
    ) -> Result<RoomBuildPlan, TowerError> {
        // Resolve the target floor once and retain its index for cost calculation and insertion.
        let floor_index = self
            .floors
            .iter()
            .position(|floor| floor.number == floor_number)
            .ok_or(TowerError::FloorNotFound(floor_number))?;

        // Lobbies are limited to ground level and positive multiples of 15.
        if room_type == RoomType::Lobby && (floor_number < 0 || floor_number % 15 != 0) {
            return Err(TowerError::LobbyOnlyAllowedOnEveryFifteenthFloor {
                floor: floor_number,
            });
        }
        if floor_number == 0 && room_type != RoomType::Lobby {
            return Err(TowerError::GroundFloorLobbyOnly);
        }

        let width = room_type.properties().width;
        // Check the room's entire horizontal footprint using a right-exclusive edge.
        let right_edge = i64::from(x) + i64::from(width);
        if right_edge > i64::from(MAX_FLOOR_WIDTH_COLUMNS) {
            return Err(TowerError::RoomExceedsFloorWidth { x, width });
        }

        // Plan segments for the whole room footprint. Ground-floor lobby segments need no support
        // below; segments on higher floors require a room or segment directly underneath.
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

        // Check existing room footprints; touching edges are allowed, shared columns are not.
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

        // Return the validated changes for the caller to price or commit.
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
    /// Only lobbies may be placed on ground level, and lobbies are restricted to ground level and
    /// positive multiples of 15. Missing floor segments are added automatically; segments above
    /// ground must have support below. The build fails if the room exceeds the floor width or
    /// overlaps another room. Rooms may overlap elevator shafts.
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
    /// tower, or a standalone floor segment occupies its column on a spanned floor. Rooms may
    /// overlap elevator shafts.
    pub fn build_elevator_shaft(
        &mut self,
        column: u32,
        start_floor: i32,
        end_floor: i32,
    ) -> Result<ElevatorId, TowerError> {
        // The shaft range includes both endpoints and must run from lower to higher floor.
        if start_floor > end_floor {
            return Err(TowerError::InvalidElevatorFloorRange {
                start_floor,
                end_floor,
            });
        }

        // Elevator columns use the same zero-based bounds as rooms and floor segments.
        if column >= MAX_FLOOR_WIDTH_COLUMNS {
            return Err(TowerError::ColumnOutOfRange(column));
        }

        // Validate both endpoints; the tower's floors are contiguous, so these bound the range.
        for floor_number in [start_floor, end_floor] {
            if !self.floors.iter().any(|floor| floor.number == floor_number) {
                return Err(TowerError::FloorNotFound(floor_number));
            }
        }

        // A shaft reserves its column on every floor in the inclusive range.
        for floor in self
            .floors
            .iter()
            .filter(|floor| start_floor <= floor.number && floor.number <= end_floor)
        {
            // Room construction adds floor segments for its footprint; those can coexist with
            // shafts. Standalone segments still reserve their column against elevator shafts.
            let segment_is_under_room = floor.rooms.iter().any(|room_id| {
                let Some(room) = self.rooms.get(*room_id) else {
                    return false;
                };
                let room_left = i64::from(room.x);
                let room_right = room_left + i64::from(room.room_type.properties().width);
                room_left <= i64::from(column) && i64::from(column) < room_right
            });
            if floor.floor_segments.contains(&column) && !segment_is_under_room {
                return Err(TowerError::FloorSegmentOverlapsElevator {
                    floor: floor.number,
                    column,
                });
            }
        }

        // Insert only after every floor in the span passes collision validation.
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
        // Use ground-floor lobbies as supports, then build the requested columns above them.
        for column in columns {
            tower
                .build_room(0, column, RoomType::Lobby)
                .expect("lobby should provide ground-floor support");
            for floor in 1..=floor_number {
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
        assert_eq!(
            tower.build_floor_segment(0, 5),
            Err(TowerError::GroundFloorLobbyOnly)
        );
        tower
            .build_room(0, 5, RoomType::Lobby)
            .expect("lobby should build on the ground floor");
        tower
            .build_floor_segment(1, 5)
            .expect("ground-floor lobby should support the next floor");
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
            .build_room(0, 8, RoomType::Lobby)
            .expect("lobby should build on the ground floor");
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
        for room_type in [RoomType::Condo, RoomType::PartyHall, RoomType::Office] {
            assert_eq!(
                tower.build_room(0, 0, room_type),
                Err(TowerError::GroundFloorLobbyOnly)
            );
        }
        let ground_floor = tower
            .floors
            .iter()
            .find(|floor| floor.number == 0)
            .expect("ground floor should exist");
        assert!(ground_floor.floor_segments.is_empty());
        assert!(ground_floor.rooms.is_empty());

        tower
            .build_room(0, 0, RoomType::Lobby)
            .expect("lobby should build on the ground floor");
        build_supported_segments(&mut tower, 1, 1..16);

        let ground_floor = tower
            .floors
            .iter()
            .find(|floor| floor.number == 0)
            .expect("ground floor should exist");
        let mut ground_segments = ground_floor.floor_segments.clone();
        ground_segments.sort_unstable();
        assert_eq!(ground_segments, (0..16).collect::<Vec<_>>());

        assert!(tower.build_room(1, 0, RoomType::Condo).is_ok());
        let first_floor = tower
            .floors
            .iter()
            .find(|floor| floor.number == 1)
            .expect("first floor should exist");
        let mut first_floor_segments = first_floor.floor_segments.clone();
        first_floor_segments.sort_unstable();
        assert_eq!(first_floor_segments, (0..16).collect::<Vec<_>>());
    }

    #[test]
    fn lobbies_are_allowed_on_ground_and_every_fifteenth_floor_above_it() {
        let mut tower = Tower::default();
        assert!(tower.build_room(0, 0, RoomType::Lobby).is_ok());

        assert!(matches!(
            tower.build_room(1, 0, RoomType::Lobby),
            Err(TowerError::LobbyOnlyAllowedOnEveryFifteenthFloor { floor: 1 })
        ));
        assert!(matches!(
            tower.build_room(-1, 0, RoomType::Lobby),
            Err(TowerError::LobbyOnlyAllowedOnEveryFifteenthFloor { floor: -1 })
        ));

        build_supported_segments(&mut tower, 15, 1..2);
        assert!(tower.build_room(15, 1, RoomType::Lobby).is_ok());
        build_supported_segments(&mut tower, 30, 2..3);
        assert!(tower.build_room(30, 2, RoomType::Lobby).is_ok());
    }

    #[test]
    fn room_can_auto_build_floor_segments_supported_by_a_room_below() {
        let mut tower = Tower::default();
        build_supported_segments(&mut tower, 1, 8..32);
        tower
            .build_room(1, 8, RoomType::PartyHall)
            .expect("room should build on its floor segments");
        let first_floor = tower
            .floors
            .iter_mut()
            .find(|floor| floor.number == 1)
            .expect("first floor should exist");
        // Remove the segment to isolate the room as the support source.
        first_floor.floor_segments.clear();

        tower
            .build_room(2, 8, RoomType::Office)
            .expect("room below should support the automatically added segment");
        assert!(
            tower
                .floors
                .iter()
                .find(|floor| floor.number == 2)
                .expect("second floor should exist")
                .floor_segments
                .contains(&8)
        );
    }

    #[test]
    fn unsupported_room_build_does_not_add_partial_floor_segments() {
        let mut tower = Tower::default();
        build_supported_segments(&mut tower, 1, 0..15);

        assert!(matches!(
            tower.build_room(2, 0, RoomType::Condo),
            Err(TowerError::FloorSegmentUnsupported {
                floor: 2,
                column: 15
            })
        ));
        assert!(
            tower
                .floors
                .iter()
                .find(|floor| floor.number == 2)
                .expect("second floor should exist")
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
            1,
            MAX_FLOOR_WIDTH_COLUMNS - condo_width..MAX_FLOOR_WIDTH_COLUMNS,
        );

        assert!(
            tower
                .build_room(1, MAX_FLOOR_WIDTH_COLUMNS - condo_width, RoomType::Condo,)
                .is_ok()
        );
        assert!(matches!(
            tower.build_room(
                1,
                MAX_FLOOR_WIDTH_COLUMNS - condo_width + 1,
                RoomType::Condo,
            ),
            Err(TowerError::RoomExceedsFloorWidth { .. })
        ));
    }

    #[test]
    fn room_build_is_rejected_if_its_columns_overlap_an_existing_room() {
        let mut tower = Tower::default();
        build_supported_segments(&mut tower, 1, 9..35);
        let existing_room_id = tower
            .build_room(1, 10, RoomType::Condo)
            .expect("first room should build");

        assert!(matches!(
            tower.build_room(1, 9, RoomType::Office),
            Err(TowerError::RoomOverlap(room_id)) if room_id == existing_room_id
        ));
        assert!(
            tower.build_room(1, 26, RoomType::Office).is_ok(),
            "rooms whose edges touch should not overlap"
        );
    }

    #[test]
    fn rooms_can_be_built_on_floors_spanned_by_an_elevator() {
        let mut tower = Tower::default();
        tower
            .build_elevator_shaft(12, 0, 1)
            .expect("elevator shaft should build across empty floors");
        tower
            .build_room(0, 12, RoomType::Lobby)
            .expect("ground-floor room should overlap the elevator shaft");
        build_supported_segments(&mut tower, 1, 13..21);
        tower
            .build_room(1, 12, RoomType::Office)
            .expect("upper-floor room should overlap the elevator shaft");

        assert!(
            tower
                .floors
                .iter()
                .find(|floor| floor.number == 1)
                .expect("first floor should exist")
                .floor_segments
                .contains(&12)
        );
    }

    #[test]
    fn elevator_can_be_built_through_a_room_and_its_floor_segments() {
        let mut tower = Tower::default();
        build_supported_segments(&mut tower, 1, 10..26);
        tower
            .build_room(1, 10, RoomType::Condo)
            .expect("room should build");

        assert!(tower.build_elevator_shaft(12, 1, 1).is_ok());
    }

    #[test]
    fn elevator_build_is_rejected_if_a_standalone_floor_segment_occupies_its_column() {
        let mut tower = Tower::default();
        tower
            .build_room(0, 12, RoomType::Lobby)
            .expect("lobby should provide support");
        tower
            .build_floor_segment(1, 12)
            .expect("standalone segment should build above lobby");

        assert!(matches!(
            tower.build_elevator_shaft(12, 1, 1),
            Err(TowerError::FloorSegmentOverlapsElevator {
                floor: 1,
                column: 12
            })
        ));
    }

    /// A room added to a floor can later be removed by its ID.
    #[test]
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
        assert_eq!(tower.rooms.len(), 16);

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
        build_supported_segments(&mut tower, 1, 0..16);
        let room_id = tower
            .build_room(1, 0, RoomType::Condo)
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
