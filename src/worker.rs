use crate::GameState;
use crate::messages::{RendererWorkerMessage, UiInput, send_message};
use crate::tower::FLOOR_SEGMENT_COST;
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
                // Keep cost validation, construction, and payment atomic with respect to other
                // state users by doing them under the same lock.
                let built_room = match game_state.lock() {
                    Ok(mut state) => {
                        // Planning validates placement and counts only the floor segments that
                        // construction would need to add.
                        let total_cost = match state.building.room_build_cost(floor, x, room_type) {
                            Ok(cost) => cost,
                            Err(error) => {
                                eprintln!("could not build room: {error:?}");
                                continue;
                            }
                        };
                        if state.economy.cash < total_cost {
                            eprintln!(
                                "insufficient cash to build room: need {total_cost}, have {}",
                                state.economy.cash
                            );
                            None
                        } else {
                            // The plan has succeeded and funds are available; mutate the tower
                            // first, then charge only after construction succeeds.
                            match state.building.build_room(floor, x, room_type) {
                                Ok(room_id) => {
                                    state.economy.cash -= total_cost;
                                    Some(RendererWorkerMessage::RoomBuilt {
                                        id: room_id,
                                        floor,
                                        x,
                                        room_type,
                                    })
                                }
                                Err(error) => {
                                    eprintln!("could not build room: {error:?}");
                                    None
                                }
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("game state lock poisoned: {error}");
                        break;
                    }
                };
                if let Some(message) = built_room {
                    send_message(&to_renderer, message);
                }
            }
            RendererWorkerMessage::UiInput(UiInput::BuildFloorSegment { floor, column }) => {
                // Standalone floor segments have a fixed cost; validate funds before asking the
                // tower to apply its placement and support rules.
                let built_segment = match game_state.lock() {
                    Ok(mut state) => {
                        if state.economy.cash < FLOOR_SEGMENT_COST {
                            eprintln!(
                                "insufficient cash to build floor segment: need {FLOOR_SEGMENT_COST}, have {}",
                                state.economy.cash
                            );
                            None
                        } else {
                            match state.building.build_floor_segment(floor, column) {
                                Ok(()) => {
                                    state.economy.cash -= FLOOR_SEGMENT_COST;
                                    Some(RendererWorkerMessage::FloorSegmentBuilt { floor, column })
                                }
                                Err(error) => {
                                    eprintln!("could not build floor segment: {error:?}");
                                    None
                                }
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("game state lock poisoned: {error}");
                        break;
                    }
                };
                if let Some(message) = built_segment {
                    send_message(&to_renderer, message);
                }
            }
            RendererWorkerMessage::UiInput(UiInput::RemoveRoom(room_id)) => {
                // Update authoritative state before telling the renderer to remove its display.
                let removed = match game_state.lock() {
                    Ok(mut state) => match state.building.remove_room(room_id) {
                        Ok(_) => true,
                        Err(error) => {
                            eprintln!("could not remove room: {error:?}");
                            false
                        }
                    },
                    Err(error) => {
                        eprintln!("game state lock poisoned: {error}");
                        break;
                    }
                };
                if removed {
                    send_message(&to_renderer, RendererWorkerMessage::RoomRemoved(room_id));
                }
            }
            RendererWorkerMessage::RoomBuilt { .. }
            | RendererWorkerMessage::FloorSegmentBuilt { .. }
            | RendererWorkerMessage::RoomRemoved(_) => {
                // These variants are worker outputs, so receiving them here indicates a
                // direction/protocol error rather than a user action.
                eprintln!("unexpected renderer output received by worker");
            }
            RendererWorkerMessage::Shutdown => break,
        }
    }

    // Signal that no further worker-to-renderer messages will be sent.
    send_message(&to_renderer, RendererWorkerMessage::Shutdown);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tower::Tower;
    use std::thread;

    fn run_worker_request(
        state: Arc<Mutex<GameState>>,
        input: UiInput,
    ) -> mpsc::Receiver<RendererWorkerMessage> {
        let (input_sender, input_receiver) = mpsc::channel();
        let (output_sender, output_receiver) = mpsc::channel();
        let worker = thread::spawn(move || run(input_receiver, state, output_sender));
        // Shut down after one request so each test can inspect the resulting state and messages.
        input_sender
            .send(RendererWorkerMessage::UiInput(input))
            .expect("worker should receive the request");
        input_sender
            .send(RendererWorkerMessage::Shutdown)
            .expect("worker should receive shutdown");
        worker.join().expect("worker should exit");
        output_receiver
    }

    #[test]
    fn building_room_deducts_its_cost_from_cash() {
        let mut building = Tower::default();
        for column in 0..16 {
            building
                .build_floor_segment(0, column)
                .expect("ground floor segment should build");
            building
                .build_floor_segment(1, column)
                .expect("ground segment should support the upper floor");
        }
        let state = Arc::new(Mutex::new(GameState {
            building,
            economy: crate::economy::Economy {
                cash: 500,
                ..Default::default()
            },
            ..Default::default()
        }));
        let (input_sender, input_receiver) = mpsc::channel();
        let (output_sender, output_receiver) = mpsc::channel();
        let worker_state = Arc::clone(&state);
        let worker = thread::spawn(move || run(input_receiver, worker_state, output_sender));

        input_sender
            .send(RendererWorkerMessage::UiInput(UiInput::BuildRoom {
                floor: 1,
                x: 0,
                room_type: crate::room::RoomType::Condo,
            }))
            .expect("worker should receive the build request");
        input_sender
            .send(RendererWorkerMessage::Shutdown)
            .expect("worker should receive shutdown");
        worker.join().expect("worker should exit");

        assert_eq!(state.lock().unwrap().economy.cash, 400);
        assert!(matches!(
            output_receiver.recv().unwrap(),
            RendererWorkerMessage::RoomBuilt { .. }
        ));
        assert!(matches!(
            output_receiver.recv().unwrap(),
            RendererWorkerMessage::Shutdown
        ));
    }

    #[test]
    fn failed_room_build_does_not_deduct_cost() {
        let state = Arc::new(Mutex::new(GameState {
            economy: crate::economy::Economy {
                cash: 500,
                ..Default::default()
            },
            ..Default::default()
        }));
        let (input_sender, input_receiver) = mpsc::channel();
        let (output_sender, output_receiver) = mpsc::channel();
        let worker_state = Arc::clone(&state);
        let worker = thread::spawn(move || run(input_receiver, worker_state, output_sender));

        input_sender
            .send(RendererWorkerMessage::UiInput(UiInput::BuildRoom {
                floor: 100,
                x: 0,
                room_type: crate::room::RoomType::Condo,
            }))
            .expect("worker should receive the build request");
        input_sender
            .send(RendererWorkerMessage::Shutdown)
            .expect("worker should receive shutdown");
        worker.join().expect("worker should exit");

        assert_eq!(state.lock().unwrap().economy.cash, 500);
        assert!(matches!(
            output_receiver.recv().unwrap(),
            RendererWorkerMessage::Shutdown
        ));
    }

    #[test]
    fn building_room_is_rejected_when_cash_is_insufficient() {
        let state = Arc::new(Mutex::new(GameState {
            economy: crate::economy::Economy {
                cash: 99,
                ..Default::default()
            },
            ..Default::default()
        }));
        let (input_sender, input_receiver) = mpsc::channel();
        let (output_sender, output_receiver) = mpsc::channel();
        let worker_state = Arc::clone(&state);
        let worker = thread::spawn(move || run(input_receiver, worker_state, output_sender));

        input_sender
            .send(RendererWorkerMessage::UiInput(UiInput::BuildRoom {
                floor: 1,
                x: 0,
                room_type: crate::room::RoomType::Condo,
            }))
            .expect("worker should receive the build request");
        input_sender
            .send(RendererWorkerMessage::Shutdown)
            .expect("worker should receive shutdown");
        worker.join().expect("worker should exit");

        let state = state.lock().unwrap();
        assert_eq!(state.economy.cash, 99);
        assert!(state.building.rooms.is_empty());
        drop(state);
        assert!(matches!(
            output_receiver.recv().unwrap(),
            RendererWorkerMessage::Shutdown
        ));
    }

    #[test]
    fn building_floor_segment_deducts_its_cost_and_checks_cash() {
        let state = Arc::new(Mutex::new(GameState {
            economy: crate::economy::Economy {
                cash: FLOOR_SEGMENT_COST,
                ..Default::default()
            },
            ..Default::default()
        }));
        let output = run_worker_request(
            Arc::clone(&state),
            UiInput::BuildFloorSegment {
                floor: 0,
                column: 7,
            },
        );

        assert_eq!(state.lock().unwrap().economy.cash, 0);
        assert!(
            state.lock().unwrap().building.floors[10]
                .floor_segments
                .contains(&7)
        );
        assert!(matches!(
            output.recv().unwrap(),
            RendererWorkerMessage::FloorSegmentBuilt {
                floor: 0,
                column: 7
            }
        ));
        assert!(matches!(
            output.recv().unwrap(),
            RendererWorkerMessage::Shutdown
        ));

        let state = Arc::new(Mutex::new(GameState {
            economy: crate::economy::Economy {
                cash: FLOOR_SEGMENT_COST - 1,
                ..Default::default()
            },
            ..Default::default()
        }));
        let output = run_worker_request(
            Arc::clone(&state),
            UiInput::BuildFloorSegment {
                floor: 0,
                column: 7,
            },
        );

        let state = state.lock().unwrap();
        assert_eq!(state.economy.cash, FLOOR_SEGMENT_COST - 1);
        assert!(state.building.floors[10].floor_segments.is_empty());
        drop(state);
        assert!(matches!(
            output.recv().unwrap(),
            RendererWorkerMessage::Shutdown
        ));
    }

    #[test]
    fn building_room_charges_only_for_missing_floor_segments() {
        let mut building = Tower::default();
        for column in 0..15 {
            building
                .build_floor_segment(0, column)
                .expect("ground floor segment should build");
        }
        let state = Arc::new(Mutex::new(GameState {
            building,
            economy: crate::economy::Economy {
                cash: 600,
                ..Default::default()
            },
            ..Default::default()
        }));
        let output = run_worker_request(
            Arc::clone(&state),
            UiInput::BuildRoom {
                floor: 0,
                x: 0,
                room_type: crate::room::RoomType::Condo,
            },
        );

        let state = state.lock().unwrap();
        assert_eq!(state.economy.cash, 0);
        assert_eq!(state.building.floors[10].floor_segments.len(), 16);
        assert_eq!(state.building.floors[10].rooms.len(), 1);
        drop(state);
        assert!(matches!(
            output.recv().unwrap(),
            RendererWorkerMessage::RoomBuilt { .. }
        ));
        assert!(matches!(
            output.recv().unwrap(),
            RendererWorkerMessage::Shutdown
        ));
    }

    #[test]
    fn room_build_with_insufficient_segment_funds_changes_nothing() {
        let state = Arc::new(Mutex::new(GameState {
            economy: crate::economy::Economy {
                cash: 8_099,
                ..Default::default()
            },
            ..Default::default()
        }));
        let output = run_worker_request(
            Arc::clone(&state),
            UiInput::BuildRoom {
                floor: 0,
                x: 0,
                room_type: crate::room::RoomType::Condo,
            },
        );

        let state = state.lock().unwrap();
        assert_eq!(state.economy.cash, 8_099);
        assert!(state.building.floors[10].floor_segments.is_empty());
        assert!(state.building.rooms.is_empty());
        drop(state);
        assert!(matches!(
            output.recv().unwrap(),
            RendererWorkerMessage::Shutdown
        ));
    }
}
