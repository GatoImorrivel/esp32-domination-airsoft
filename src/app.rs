use std::{
    ffi::CStr,
    sync::Arc,
    time::{Duration, Instant},
};

use esp_idf_svc::{
    eventloop::{
        EspEvent, EspEventDeserializer, EspEventPostData, EspEventSerializer, EspEventSource,
        EspSystemEventLoop,
    },
    hal::delay::FreeRtos,
    http::server::EspHttpServer,
    wifi::EspWifi,
};

use crate::{
    assets::{LOW_HONOR_SOUND, PLUH_SOUND},
    bt::BluetoothAudio,
    wifi::Wifi,
};

#[derive(Debug, Clone, Copy)]
pub struct GameState {
    active: bool,
    current_team: Option<Team>,
    last_tick: Option<Instant>,
    team_red_time: Duration,
    team_blue_time: Duration,
    time_to_win: Duration,
}

impl Default for GameState {
    fn default() -> Self {
        GameState::new(Duration::from_secs(10))
    }
}

impl GameState {
    pub fn new(time_to_win: Duration) -> Self {
        Self {
            active: false,
            current_team: None,
            last_tick: None,
            team_red_time: Duration::ZERO,
            team_blue_time: Duration::ZERO,
            time_to_win,
        }
    }

    /// Start or restart the game
    pub fn start(&mut self) {
        self.active = true;
        self.current_team = None;
        self.last_tick = Some(Instant::now());
        self.team_red_time = Duration::ZERO;
        self.team_blue_time = Duration::ZERO;
    }

    /// Stop the game (no more accumulation)
    pub fn stop(&mut self) {
        self.tick();
        self.active = false;
        self.current_team = None;
        self.last_tick = None;
    }

    /// Handle a button press
    pub fn button_press(&mut self, team: Team) {
        if !self.active {
            return;
        }

        // First, account for time so far
        self.tick();

        // Switch ownership
        self.current_team = Some(team);
    }

    /// Call this periodically (e.g. every 50–100 ms)
    pub fn tick(&mut self) {
        if !self.active {
            return;
        }

        let now = Instant::now();
        let Some(last) = self.last_tick else {
            self.last_tick = Some(now);
            return;
        };

        let delta = now.duration_since(last);

        if let Some(owner) = self.current_team {
            match owner {
                Team::Blue => self.team_blue_time += delta,
                Team::Red => self.team_red_time += delta,
            }
        }

        self.last_tick = Some(now);
    }

    /// Check if someone won
    pub fn winner(&self) -> Option<Team> {
        if self.team_blue_time >= self.time_to_win {
            Some(Team::Red)
        } else if self.team_red_time >= self.time_to_win {
            Some(Team::Blue)
        } else {
            None
        }
    }

    /// Expose current scores (for UI / WS)
    pub fn scores(&self) -> (Duration, Duration) {
        (self.team_blue_time, self.team_red_time)
    }

    /// Who currently owns the point
    pub fn current_team(&self) -> Option<Team> {
        self.current_team
    }
}

#[derive(Debug)]
pub struct App {
    app_state: AppState,
    current_game: GameState,
}

impl Default for App {
    fn default() -> Self {
        Self {
            app_state: AppState::Setup,
            current_game: GameState::default(),
        }
    }
}

impl App {
    pub fn run(mut self, mut wifi: Wifi, event_loop: EspSystemEventLoop) {
        let mut server = EspHttpServer::new(&esp_idf_svc::http::server::Configuration {
            ..Default::default()
        })
        .unwrap();

        let start_el = event_loop.clone();
        server.fn_handler("/game/start", esp_idf_svc::http::Method::Post, move |req| {
            start_el.post::<AppEvent>(&AppEvent::StartGame, 0).unwrap();

            req.into_ok_response()
                .unwrap()
                .write("".as_bytes())
                .map(|_| ())
        }).unwrap();

        let stop_el = event_loop.clone();
        server.fn_handler("/game/end", esp_idf_svc::http::Method::Post, move |req| {
            stop_el.post::<AppEvent>(&AppEvent::EndGame { winner: None }, 0).unwrap();
            req.into_ok_response()
                .unwrap()
                .write("".as_bytes())
                .map(|_| ())
        }).unwrap();

        esp_idf_svc::hal::task::block_on(core::pin::pin!(async move {
            wifi.ap_mode().await.unwrap();
            let mut sub = event_loop.subscribe_async::<AppEvent>().unwrap();
            let bt = BluetoothAudio::get().unwrap();
            loop {
                let ev = sub.recv().await.unwrap();

                match ev {
                    AppEvent::StartGame => {
                        if !self.current_game.active {
                            let mut game = GameState::new(Duration::from_secs(10));
                            game.start();
                            self.current_game = game;
                        }
                    }
                    AppEvent::EndGame { winner } => {
                        if !self.current_game.active {
                            return;
                        }

                        self.current_game.stop();

                        if let Some(winner) = winner {
                            log::info!("Winner is {winner:?}");
                        } else {
                            log::info!("Game ended with no winner");
                        }
                    }
                    AppEvent::ButtonPress(team) => {
                        if !self.current_game.active {
                            return;
                        }

                        match team {
                            Team::Blue => {
                                log::info!("Blue team pressed");
                                bt.play_audio(PLUH_SOUND);
                            }
                            Team::Red => {
                                log::info!("Red team pressed");
                                bt.play_audio(LOW_HONOR_SOUND);
                            }
                        }
                        self.current_game.button_press(team);
                    }
                }
            }
        }));
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AppState {
    Setup,
    Idle,
    InGame,
}

#[derive(Debug, Clone, Copy)]
pub enum Team {
    Red,
    Blue,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug)]
pub enum AppEvent {
    ButtonPress(Team),
    StartGame,
    EndGame { winner: Option<Team> },
}

unsafe impl EspEventSource for AppEvent {
    #[allow(clippy::manual_c_str_literals)]
    fn source() -> Option<&'static CStr> {
        // String should be unique across the whole project and ESP IDF
        Some(CStr::from_bytes_with_nul(b"DOMINACAO-SERVICE\0").unwrap())
    }
}

impl EspEventSerializer for AppEvent {
    type Data<'a> = AppEvent;

    fn serialize<F, R>(event: &Self::Data<'_>, f: F) -> R
    where
        F: FnOnce(&EspEventPostData) -> R,
    {
        f(&unsafe { EspEventPostData::new(Self::source().unwrap(), Self::event_id(), event) })
    }
}

impl EspEventDeserializer for AppEvent {
    type Data<'a> = AppEvent;

    fn deserialize<'a>(data: &EspEvent<'a>) -> Self::Data<'a> {
        *unsafe { data.as_payload::<AppEvent>() }
    }
}
