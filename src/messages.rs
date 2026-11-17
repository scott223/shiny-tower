use crate::room::{RoomId, RoomType};
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
        x: u32,
        room_type: RoomType,
    },
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
    /// Request a room be built on a floor.
    BuildRoom {
        floor: i32,
        /// The room's column position on the floor.
        x: u32,
        room_type: RoomType,
    },
    /// Request a floor segment be built at a column.
    BuildFloorSegment { floor: i32, column: u32 },
    /// Request a room be destroyed by ID.
    RemoveRoom(RoomId),
}

/// Simulation updates sent directly from the simulation thread to the renderer.
#[derive(Debug)]
pub enum SimulationRendererMessage {
    /// Indicates that the simulation thread completed a tick.
    Tick(u64),
    /// Stops the renderer's simulation-update receiver.
    Shutdown,
}

/// Sends a typed message and reports if its receiving channel has closed.
pub fn send_message<T: std::fmt::Debug>(sender: &Sender<T>, message: T) {
    // Report disconnections instead of silently dropping a message.
    if let Err(error) = sender.send(message) {
        eprintln!("message channel disconnected: {error}");
    }
}
