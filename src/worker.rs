use crate::GameState;
use crate::messages::{RendererWorkerMessage, UiInput, send_message};
use std::sync::{Arc, Mutex, mpsc};

/// Handles renderer/UI requests and performs worker-owned state changes.
pub fn run(
    receiver: mpsc::Receiver<RendererWorkerMessage>,
    game_state: Arc<Mutex<GameState>>,
    to_renderer: mpsc::Sender<RendererWorkerMessage>,
) {
    // Process UI messages in order so worker-owned state changes stay ordered.
    while let Ok(message) = receiver.recv() {
        match message {
            RendererWorkerMessage::UiInput(UiInput::BuildRoom {
                floor,
                x,
                room_type,
            }) => {
                // Notify the renderer only after the room was successfully added to the tower.
                let built_room = match game_state.lock() {
                    Ok(mut state) => match state.building.build_room(floor, x, room_type) {
                        Ok(room) => Some(RendererWorkerMessage::RoomBuilt {
                            id: room.id,
                            floor,
                            x: room.x,
                            room_type: room.room_type,
                        }),
                        Err(error) => {
                            eprintln!("could not build room: {error:?}");
                            None
                        }
                    },
                    Err(error) => {
                        eprintln!("game state lock poisoned: {error}");
                        break;
                    }
                };
                if let Some(message) = built_room {
                    send_message(&to_renderer, message);
                }
            }
            RendererWorkerMessage::UiInput(UiInput::RemoveRoom(room_id)) => {
                // Remove the room from state before emitting its renderer update.
                let removed = match game_state.lock() {
                    Ok(mut state) => state.building.remove_room(room_id).is_some(),
                    Err(error) => {
                        eprintln!("game state lock poisoned: {error}");
                        break;
                    }
                };
                if removed {
                    send_message(&to_renderer, RendererWorkerMessage::RoomRemoved(room_id));
                } else {
                    eprintln!("could not remove unknown room: {room_id:?}");
                }
            }
            RendererWorkerMessage::RoomBuilt { .. } | RendererWorkerMessage::RoomRemoved(_) => {
                eprintln!("unexpected renderer output received by worker");
            }
            RendererWorkerMessage::Shutdown => break,
        }
    }

    // Signal that no further worker-to-renderer messages will be sent.
    send_message(&to_renderer, RendererWorkerMessage::Shutdown);
}
