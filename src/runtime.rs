use crate::{GameState, renderer, simulation, worker};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

/// Creates the shared state and communication channels, then starts the threads.
pub fn run() {
    // Both state-mutating threads share the same state behind a mutex.
    let game_state = Arc::new(Mutex::new(GameState::default()));
    // Keep UI input, worker output, and simulation ticks on separate channels.
    let (renderer_to_worker_sender, renderer_to_worker_receiver) = mpsc::channel();
    let (worker_to_renderer_sender, worker_to_renderer_receiver) = mpsc::channel();
    let (simulation_to_renderer_sender, simulation_to_renderer_receiver) = mpsc::channel();

    // The simulation advances ticks independently and publishes them to the renderer.
    let simulation_state = Arc::clone(&game_state);
    let simulation_thread =
        thread::spawn(move || simulation::run(simulation_state, simulation_to_renderer_sender));

    // The worker applies UI requests and publishes resulting state changes.
    let worker_state = Arc::clone(&game_state);
    let worker_to_renderer = worker_to_renderer_sender.clone();
    let _worker_thread = thread::spawn(move || {
        worker::run(
            renderer_to_worker_receiver,
            worker_state,
            worker_to_renderer,
        )
    });

    // The renderer consumes both worker updates and simulation ticks.
    let _renderer_thread = thread::spawn(move || {
        renderer::run(
            renderer_to_worker_sender,
            worker_to_renderer_receiver,
            simulation_to_renderer_receiver,
        )
    });

    // Keep the runtime alive for as long as the simulation loop is running.
    simulation_thread
        .join()
        .expect("simulation thread panicked");
}
