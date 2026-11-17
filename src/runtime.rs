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
    let (speed_request_sender, speed_request_receiver) = mpsc::channel();

    // The simulation advances ticks independently and publishes them to the renderer.
    let simulation_state = Arc::clone(&game_state);
    let simulation_thread = thread::spawn(move || {
        simulation::run(
            simulation_state,
            speed_request_receiver,
            simulation_to_renderer_sender,
        )
    });

    // The worker applies UI requests and publishes resulting state changes.
    let worker_state = Arc::clone(&game_state);
    let worker_to_renderer = worker_to_renderer_sender.clone();
    let worker_to_simulation = speed_request_sender;
    let worker_thread = thread::spawn(move || {
        worker::run(
            renderer_to_worker_receiver,
            worker_state,
            worker_to_renderer,
            worker_to_simulation,
        )
    });

    // Bevy's window event loop must run on the main thread on macOS.
    renderer::run(
        renderer_to_worker_sender,
        worker_to_renderer_receiver,
        simulation_to_renderer_receiver,
    );

    // Renderer exit signals the worker, which disconnects the simulation speed channel.
    simulation_thread
        .join()
        .expect("simulation thread panicked");
    worker_thread.join().expect("worker thread panicked");
}
