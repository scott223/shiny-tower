use crate::GameState;
use crate::messages::{SimulationRendererMessage, send_message};
use std::sync::{Arc, Mutex, mpsc::Sender};
use std::thread;
use std::time::Duration;

const TICK_INTERVAL: Duration = Duration::from_millis(100);

/// Advances the shared simulation tick and publishes each tick to the renderer.
///
/// The lock is held only while changing and reading the tick counter.
pub fn run(game_state: Arc<Mutex<GameState>>, to_renderer: Sender<SimulationRendererMessage>) {
    loop {
        // Pace updates independently of renderer frame timing.
        thread::sleep(TICK_INTERVAL);
        // Update the shared tick while holding the state lock, then release it before sending.
        let tick = match game_state.lock() {
            Ok(mut state) => {
                state.simulation_tick = state.simulation_tick.saturating_add(1);
                state.simulation_tick
            }
            Err(error) => {
                eprintln!("game state lock poisoned: {error}");
                break;
            }
        };
        send_message(&to_renderer, SimulationRendererMessage::Tick(tick));
    }
}
