use crate::room::{RoomId, RoomType};
use crate::steps::StepsId;
use std::sync::mpsc::Sender;

/// Messages sent between the renderer and the worker handling UI requests.
#[derive(Debug)]
pub enum RendererWorkerMessage {
    /// Input from the renderer/UI that the worker should forward for processing.
    UiInput(UiInput),
    /// Sent by the worker after it successfully builds a room.
    RoomBuilt {
        id: RoomId,
        floor: i32,
        /// Leftmost column occupied by the room.
        x: u32,
        room_type: RoomType,
    },
    /// Sent when a room build request could not be completed.
    RoomBuildFailed { error: String },
    /// Sent by the worker after it successfully builds steps.
    StepsBuilt { id: StepsId, floor: i32, x: u32 },
    /// Sent when a steps build request could not be completed.
    StepsBuildFailed { error: String },
    /// Sent by the worker after it successfully builds a floor segment.
    FloorSegmentBuilt { floor: i32, column: u32 },
    /// Sent by the worker after it successfully destroys a room.
    RoomRemoved(RoomId),
    /// Stops the receiving renderer or worker loop.
    Shutdown,
}

/// UI requests handled by the worker.
#[derive(Debug)]
pub enum UiInput {
    /// Set how quickly simulation ticks advance.
    SetSimulationSpeed(SimulationSpeed),
    /// Request a room be built on a floor.
    BuildRoom {
        floor: i32,
        /// Leftmost column occupied by the room.
        x: u32,
        room_type: RoomType,
    },
    /// Request steps be built across the selected floor and the one above.
    BuildSteps { floor: i32, x: u32 },
    /// Request a floor segment be built at a column.
    BuildFloorSegment { floor: i32, column: u32 },
    /// Request a room be destroyed by ID.
    RemoveRoom(RoomId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Simulation tick rate requested by the renderer.
pub enum SimulationSpeed {
    Paused,
    OneX,
    ThreeX,
    FiveX,
}

/// Simulation updates sent directly from the simulation thread to the renderer.
#[derive(Debug)]
pub enum SimulationRendererMessage {
    /// Indicates that the simulation thread completed a tick with its corresponding date and time.
    Tick {
        tick: u64,
        date: SimulationDate,
        time: SimulationTime,
        cash: u64,
    },
    /// Stops the renderer's simulation-update receiver.
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Time of day derived from the simulation tick.
pub struct SimulationTime {
    pub hour: u8,
    pub minute: u8,
    pub period: TimeOfDayPeriod,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Period of the day associated with a simulation time.
pub enum TimeOfDayPeriod {
    Day,
    Dusk,
    Dawn,
    Night,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Position of a day within each three-day quarter.
pub enum SimulationDayType {
    Weekday1,
    Weekday2,
    Weekend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Calendar date derived from the running day counter.
pub struct SimulationDate {
    /// Zero-based day counter suitable for saving and restoring the date.
    pub day_counter: u64,
    /// Day type within the quarter.
    pub day_type: SimulationDayType,
    /// One-based quarter within the year.
    pub quarter: u8,
    /// One-based year number.
    pub year: u64,
}

/// Sends a typed message and reports if its receiving channel has closed.
pub fn send_message<T: std::fmt::Debug>(sender: &Sender<T>, message: T) {
    // Report disconnections instead of silently dropping a message.
    if let Err(error) = sender.send(message) {
        eprintln!("message channel disconnected: {error}");
    }
}
