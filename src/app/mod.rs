mod game;

use std::{
    fmt::Debug,
    sync::{Arc, OnceLock, mpsc}, time::Duration,
};

use anyhow::anyhow;
use esp_idf_svc::hal::delay::FreeRtos;
use game::GameState;

pub use game::{Scores, Team};

use crate::{
    assets::{BLUE_TEAM_CAPTURE_SOUND, RED_TEAM_CAPTURE_SOUND},
    hardware::{bt::BluetoothAudio, wifi::Wifi},
};

pub enum AppEvent {
    Command(Box<dyn FnOnce(&mut App) + Send>),
    Query(Box<dyn FnOnce(&App) + Send>),
}

#[derive(Debug, Clone, Copy)]
pub enum AppState {
    Setup,
    Idle,
    InGame,
}

#[derive(Debug)]
pub struct App {
    app_state: AppState,
    current_game: GameState,
    receiver: mpsc::Receiver<AppEvent>,
    sender: mpsc::Sender<AppEvent>,
    wifi: Wifi,
    bluetooth_audio: Arc<BluetoothAudio>,
}

impl App {
    pub fn init(wifi: Wifi, bt: Arc<BluetoothAudio>) -> Self {
        let (tx, rx) = mpsc::channel::<AppEvent>();
        let app = Self {
            app_state: AppState::Setup,
            current_game: GameState::default(),
            receiver: rx,
            sender: tx,
            wifi,
            bluetooth_audio: bt,
        };
        APP_CLIENT.set(app.client()).unwrap();
        app
    }

    pub async fn run<F: Fn(&AppClient) -> () + Send + 'static>(mut self, routine: F) {
        let client = self.client();
        std::thread::spawn(move || {
            loop {
                routine(&client);
                // Yield for a little
                FreeRtos::delay_ms(10);
            }
        });
        loop {
            if self.current_game.active() {
                self.current_game.tick();
            }

            while let Ok(event) = self.receiver.try_recv() {
                match event {
                    AppEvent::Command(func) => {
                        func(&mut self);
                    }
                    AppEvent::Query(func) => {
                        func(&self);
                    }
                }
            }

            // Yield for a little
            FreeRtos::delay_ms(10);
        }
    }

    pub fn client(&self) -> AppClient {
        AppClient {
            bus: AppBus {
                sender: self.sender.clone(),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct AppBus {
    sender: mpsc::Sender<AppEvent>,
}

impl AppBus {
    pub fn query<R: Send + 'static, F: FnOnce(&App) -> R + Send + 'static>(
        &self,
        action: F,
    ) -> anyhow::Result<R> {
        let (tx, rx) = mpsc::channel();

        let function = move |app: &App| {
            let resp = action(app);
            let send_result = tx.send(resp);
            if send_result.is_err() {
                log::error!("Failed to send event");
            }
        };

        let send_result = self.sender.send(AppEvent::Query(Box::new(function)));
        if send_result.is_err() {
            return Err(anyhow!("Failed to send event"));
        }

        let response = rx.recv_timeout(Duration::from_secs(5))?;

        Ok(response)
    }

    pub fn command<F: FnOnce(&mut App) -> anyhow::Result<()> + Send + 'static>(
        &self,
        action: F,
    ) -> anyhow::Result<()> {
        let (tx, rx) = mpsc::channel();

        let function = move |app: &mut App| {
            let resp = action(app);
            tx.send(resp).unwrap_or_else(|_| log::error!("Failed to send event"));
        };

        let send_result = self.sender.send(AppEvent::Command(Box::new(function)));
        if send_result.is_err() {
            return Err(anyhow!("Failed to send event"));
        }

        let response = rx.recv_timeout(Duration::from_secs(5))?;

        response
    }
}

#[derive(Clone, Debug)]
pub struct AppClient {
    bus: AppBus,
}

impl AppClient {
    pub fn start_game(&self) -> anyhow::Result<()> {
        self.bus.command(|app| {
            if app.current_game.active() {
                app.current_game.start();
            }
            Ok(())
        })?;

        Ok(())
    }

    pub fn team_press(&self, team: Team) -> anyhow::Result<()> {
        log::info!("Team press {team:#?}");
        self.bus.command(move |app| {
            match team {
                Team::Blue => {
                    app.bluetooth_audio.play_audio(BLUE_TEAM_CAPTURE_SOUND);
                }
                Team::Red => {
                    app.bluetooth_audio.play_audio(RED_TEAM_CAPTURE_SOUND);
                }
            }
            Ok(())
        })?;
        Ok(())
    }

    pub fn get() -> AppClient {
        let app_client = APP_CLIENT.get().expect("No app client initialized");

        app_client.clone()
    }
}

static APP_CLIENT: OnceLock<AppClient> = OnceLock::new();
