mod game;
mod server;

use std::{
    ffi::CStr, io::Repeat, time::Duration
};

use esp_idf_svc::{
    eventloop::{
        EspEvent, EspEventDeserializer, EspEventPostData, EspEventSerializer, EspEventSource,
        EspSystemEventLoop,
    },
    http::server::EspHttpServer,
};

use crate::{
    app::server::{HttpServer, Response}, assets::{BLUE_TEAM_CAPTURE_SOUND, RED_TEAM_CAPTURE_SOUND}, hardware::{bt::BluetoothAudio, wifi::Wifi}
};

use game::GameState;


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
        let mut server = HttpServer::new();

        let start_el = event_loop.clone();
        server.post::<_, (), _>("/game/start", move |_| {
            start_el.post::<AppEvent>(&AppEvent::StartGame, 0).unwrap();
            Response::ok()
        });

        let stop_el = event_loop.clone();
        server.post::<_, (), _>("/game/end", move |_| {
            stop_el.post::<AppEvent>(&AppEvent::EndGame { winner: None }, 0).unwrap();
            Response::ok()
        });

        esp_idf_svc::hal::task::block_on(core::pin::pin!(async move {
            wifi.ap_mode().await.unwrap();
            let mut sub = event_loop.subscribe_async::<AppEvent>().unwrap();
            let bt = BluetoothAudio::get().unwrap();
            loop {
                let ev = sub.recv().await.unwrap();

                match ev {
                    AppEvent::StartGame => {
                        if !self.current_game.active() {
                            let mut game = GameState::new(Duration::from_secs(10));
                            game.start();
                            self.current_game = game;
                        }
                    }
                    AppEvent::EndGame { winner } => {
                        if !self.current_game.active() {
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
                        if !self.current_game.active() {
                            return;
                        }

                        match team {
                            Team::Blue => {
                                log::info!("Blue team pressed");
                                bt.play_audio(BLUE_TEAM_CAPTURE_SOUND);
                            }
                            Team::Red => {
                                log::info!("Red team pressed");
                                bt.play_audio(RED_TEAM_CAPTURE_SOUND);
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
