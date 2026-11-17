use crate::GameState;
use crate::messages::{
    SimulationDate, SimulationDayType, SimulationRendererMessage, SimulationSpeed, SimulationTime,
    TimeOfDayPeriod, send_message,
};
use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, RecvTimeoutError, Sender},
};
use std::time::Duration;

/// Simulation tick cadence, independent of how quickly the renderer draws frames.
const TICK_INTERVAL: Duration = Duration::from_millis(60);
const TICKS_PER_DAY: u64 = 2_600;
const DAYS_PER_QUARTER: u64 = 3;
const DAYS_PER_YEAR: u64 = 12;
const TICKS_PER_QUARTER: u64 = TICKS_PER_DAY * DAYS_PER_QUARTER;

// Entries are (start tick, end tick, start minute, end minute, period); tick ranges are
// start-inclusive and end-exclusive. The seven intervals map to 07:00–12:00 Day,
// 12:00–12:30 Day, 12:30–13:00 Day, 13:00–17:00 Day, 17:00–21:00 Dusk,
// 21:00–01:00 Night, and 01:00–07:00 Night.
const DAY_SCHEDULE: [(u64, u64, u64, u64, TimeOfDayPeriod); 7] = [
    (0, 400, 420, 720, TimeOfDayPeriod::Day), // 07:00–12:00 Day
    (400, 800, 720, 750, TimeOfDayPeriod::Day), // 12:00–12:30 Day
    (800, 1_200, 750, 780, TimeOfDayPeriod::Day), // 12:30–13:00 Day
    (1_200, 1_600, 780, 1_020, TimeOfDayPeriod::Day), // 13:00–17:00 Day
    (1_600, 2_000, 1_020, 1_260, TimeOfDayPeriod::Dusk), // 17:00–21:00 Dusk
    (2_000, 2_400, 1_260, 1_500, TimeOfDayPeriod::Night), // 21:00–01:00 Night
    (2_400, 2_600, 60, 420, TimeOfDayPeriod::Night), // 01:00–07:00 Night
];

#[derive(Debug, PartialEq, Eq)]
enum QuarterlyProfitError {
    TotalOverflow,
    CashOverflow,
}

fn time_of_day(tick: u64) -> SimulationTime {
    let tick_in_day = tick % TICKS_PER_DAY;
    let (interval_start, interval_end, start_minute, end_minute, period) = DAY_SCHEDULE
        .iter()
        .copied()
        .find(|(start, end, _, _, _)| *start <= tick_in_day && tick_in_day < *end)
        .expect("every tick in a day belongs to a schedule interval");

    let elapsed_ticks = tick_in_day - interval_start;
    let interval_ticks = interval_end - interval_start;
    let elapsed_minutes = elapsed_ticks * (end_minute - start_minute) / interval_ticks;
    let minute_of_day = (start_minute + elapsed_minutes) % (24 * 60);
    SimulationTime {
        hour: (minute_of_day / 60) as u8,
        minute: (minute_of_day % 60) as u8,
        period,
    }
}

fn date_at_tick(tick: u64) -> SimulationDate {
    let day_counter = tick / TICKS_PER_DAY;
    let day_in_year = day_counter % DAYS_PER_YEAR;
    let day_in_quarter = day_in_year % DAYS_PER_QUARTER;
    let day_type = match day_in_quarter {
        0 => SimulationDayType::Weekday1,
        1 => SimulationDayType::Weekday2,
        _ => SimulationDayType::Weekend,
    };

    SimulationDate {
        day_counter,
        day_type,
        quarter: (day_in_year / DAYS_PER_QUARTER + 1) as u8,
        year: day_counter / DAYS_PER_YEAR + 1,
    }
}

fn tick_interval(speed: SimulationSpeed) -> Option<Duration> {
    match speed {
        SimulationSpeed::Paused => None,
        SimulationSpeed::OneX => Some(TICK_INTERVAL),
        SimulationSpeed::ThreeX => Some(TICK_INTERVAL / 3),
        SimulationSpeed::FiveX => Some(TICK_INTERVAL / 5),
    }
}

fn apply_quarterly_room_profit(
    state: &mut GameState,
    tick: u64,
) -> Result<(), QuarterlyProfitError> {
    if tick == 0 || tick % TICKS_PER_QUARTER != 0 {
        return Ok(());
    }

    let total_profit = state
        .building
        .rooms
        .values()
        .try_fold(0_u64, |total, room| {
            total.checked_add(room.room_type.properties().profit_quarterly)
        })
        .ok_or(QuarterlyProfitError::TotalOverflow)?;
    let updated_cash = state
        .economy
        .cash
        .checked_add(total_profit)
        .ok_or(QuarterlyProfitError::CashOverflow)?;

    state.economy.cash = updated_cash;
    Ok(())
}

/// Advances the shared simulation tick and publishes each tick to the renderer.
///
/// The lock is held only while changing and reading the tick counter and cash balance.
pub fn run(
    game_state: Arc<Mutex<GameState>>,
    from_renderer: Receiver<SimulationSpeed>,
    to_renderer: Sender<SimulationRendererMessage>,
) {
    let mut speed = SimulationSpeed::OneX;
    loop {
        // A paused simulation waits for a new speed request instead of polling or advancing.
        match tick_interval(speed) {
            None => match from_renderer.recv() {
                Ok(new_speed) => {
                    speed = new_speed;
                    continue;
                }
                Err(_) => break,
            },
            Some(interval) => match from_renderer.recv_timeout(interval) {
                Ok(new_speed) => {
                    speed = new_speed;
                    continue;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            },
        }

        // Saturation keeps the counter from wrapping back to zero after u64::MAX ticks.
        // Release the state lock before publishing so a slow renderer cannot block state access.
        let (tick, cash, profit_result) = match game_state.lock() {
            Ok(mut state) => {
                state.simulation_tick = state.simulation_tick.saturating_add(1);
                let tick = state.simulation_tick;
                let profit_result = apply_quarterly_room_profit(&mut state, tick);
                (tick, state.economy.cash, profit_result)
            }
            Err(error) => {
                eprintln!("game state lock poisoned: {error}");
                break;
            }
        };
        if let Err(error) = profit_result {
            eprintln!("could not apply quarterly room profit: {error:?}");
        }
        send_message(
            &to_renderer,
            SimulationRendererMessage::Tick {
                tick,
                date: date_at_tick(tick),
                time: time_of_day(tick),
                cash,
            },
        );
    }

    send_message(&to_renderer, SimulationRendererMessage::Shutdown);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    #[test]
    fn tick_schedule_maps_day_boundaries_and_wraps() {
        let boundaries = [
            (0, 7, 0),
            (400, 12, 0),
            (800, 12, 30),
            (1_200, 13, 0),
            (1_600, 17, 0),
            (2_000, 21, 0),
            (2_400, 1, 0),
            (2_600, 7, 0),
        ];

        for (tick, hour, minute) in boundaries {
            let time = time_of_day(tick);
            assert_eq!((time.hour, time.minute), (hour, minute));
        }
    }

    #[test]
    fn simulation_speeds_have_expected_tick_intervals() {
        assert_eq!(tick_interval(SimulationSpeed::Paused), None);
        assert_eq!(tick_interval(SimulationSpeed::OneX), Some(TICK_INTERVAL));
        assert_eq!(
            tick_interval(SimulationSpeed::ThreeX),
            Some(TICK_INTERVAL / 3)
        );
        assert_eq!(
            tick_interval(SimulationSpeed::FiveX),
            Some(TICK_INTERVAL / 5)
        );
    }

    #[test]
    fn paused_simulation_waits_for_speed_request_before_advancing() {
        let game_state = Arc::new(Mutex::new(GameState::default()));
        let (speed_sender, speed_receiver) = mpsc::channel();
        let (renderer_sender, renderer_receiver) = mpsc::channel();
        let simulation = thread::spawn(move || {
            run(game_state, speed_receiver, renderer_sender);
        });

        speed_sender.send(SimulationSpeed::Paused).unwrap();
        thread::sleep(Duration::from_millis(60));
        assert!(matches!(
            renderer_receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));

        speed_sender.send(SimulationSpeed::FiveX).unwrap();
        assert!(matches!(
            renderer_receiver.recv_timeout(Duration::from_millis(100)),
            Ok(SimulationRendererMessage::Tick {
                tick: 1,
                cash: 2_000_000,
                ..
            })
        ));
        drop(speed_sender);
        simulation.join().unwrap();
        assert!(matches!(
            renderer_receiver.recv().unwrap(),
            SimulationRendererMessage::Shutdown
        ));
    }

    #[test]
    fn tick_schedule_interpolates_between_boundaries() {
        assert_eq!(
            time_of_day(200),
            SimulationTime {
                hour: 9,
                minute: 30,
                period: TimeOfDayPeriod::Day,
            }
        );
        assert_eq!(
            time_of_day(2_500),
            SimulationTime {
                hour: 4,
                minute: 0,
                period: TimeOfDayPeriod::Night,
            }
        );
    }

    #[test]
    fn time_of_day_period_follows_day_dusk_and_night_ranges() {
        let periods = [
            (0, TimeOfDayPeriod::Day),
            (1_599, TimeOfDayPeriod::Day),
            (1_600, TimeOfDayPeriod::Dusk),
            (2_000, TimeOfDayPeriod::Night),
            (2_599, TimeOfDayPeriod::Night),
        ];

        for (tick, period) in periods {
            assert_eq!(time_of_day(tick).period, period);
        }
    }

    #[test]
    fn calendar_uses_three_day_quarters_and_twelve_day_years() {
        assert_eq!(TICKS_PER_QUARTER, 7_800);
        assert_eq!(
            date_at_tick(0),
            SimulationDate {
                day_counter: 0,
                day_type: SimulationDayType::Weekday1,
                quarter: 1,
                year: 1,
            }
        );
        assert_eq!(
            date_at_tick(2_600),
            SimulationDate {
                day_counter: 1,
                day_type: SimulationDayType::Weekday2,
                quarter: 1,
                year: 1,
            }
        );
        assert_eq!(
            date_at_tick(5_200),
            SimulationDate {
                day_counter: 2,
                day_type: SimulationDayType::Weekend,
                quarter: 1,
                year: 1,
            }
        );
        assert_eq!(
            date_at_tick(TICKS_PER_QUARTER),
            SimulationDate {
                day_counter: 3,
                day_type: SimulationDayType::Weekday1,
                quarter: 2,
                year: 1,
            }
        );
    }

    #[test]
    fn room_profits_are_credited_at_quarter_end_only() {
        let mut state = GameState::default();
        state
            .building
            .rooms
            .insert(crate::room::Room::new(0, crate::room::RoomType::Office));
        state
            .building
            .rooms
            .insert(crate::room::Room::new(10, crate::room::RoomType::Condo));
        let initial_cash = state.economy.cash;

        apply_quarterly_room_profit(&mut state, TICKS_PER_QUARTER - 1)
            .expect("non-boundary tick should not pay profit");
        assert_eq!(state.economy.cash, initial_cash);

        apply_quarterly_room_profit(&mut state, TICKS_PER_QUARTER)
            .expect("quarter boundary should pay profit");
        assert_eq!(
            state.economy.cash,
            initial_cash
                + crate::room::RoomType::Office.properties().profit_quarterly
                + crate::room::RoomType::Condo.properties().profit_quarterly
        );
    }

    #[test]
    fn quarterly_profit_overflow_does_not_change_cash() {
        let mut state = GameState::default();
        state.economy.cash = u64::MAX;
        state
            .building
            .rooms
            .insert(crate::room::Room::new(0, crate::room::RoomType::Office));

        assert_eq!(
            apply_quarterly_room_profit(&mut state, TICKS_PER_QUARTER),
            Err(QuarterlyProfitError::CashOverflow)
        );
        assert_eq!(state.economy.cash, u64::MAX);
    }
}
