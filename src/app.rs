use std::{ffi::CStr, sync::Arc};

use esp_idf_svc::{eventloop::{
    EspEvent, EspEventDeserializer, EspEventPostData, EspEventSerializer, EspEventSource,
    EspSystemEventLoop,
}, hal::delay::FreeRtos, http::server::EspHttpServer, wifi::EspWifi};

use crate::{
    assets::{LOW_HONOR_SOUND, PLUH_SOUND},
    bt::BluetoothAudio, wifi::Wifi,
};

#[derive(Debug)]
pub struct App {
    current_team: Option<Team>,
    app_state: AppState
}

impl Default for App {
    fn default() -> Self {
        Self {
            current_team: None,
            app_state: AppState::Setup
        }
    }
}

impl App {
    pub fn run(self, mut wifi: Wifi, event_loop: EspSystemEventLoop) {
        let mut server = EspHttpServer::new(&esp_idf_svc::http::server::Configuration {
            ..Default::default()
        }).unwrap();

        server.fn_handler("/discover", esp_idf_svc::http::Method::Get, move |req| {
            let bt = BluetoothAudio::get().unwrap();

            bt.start_discovery().unwrap();

            FreeRtos::delay_ms(10_000);

            bt.stop_discovery().unwrap();

            let devices = bt.discovered_devices().read().unwrap().iter().map(|d| {
                format!("{d}")
            }).collect::<Vec<_>>();

            req.into_ok_response().unwrap().write(format!("Hello world {devices:#?}").as_bytes()).map(|_| ())
        }).unwrap();

        esp_idf_svc::hal::task::block_on(core::pin::pin!(async move {
            wifi.ap_mode().await.unwrap();
            let mut sub = event_loop.subscribe_async::<AppEvent>().unwrap();
            let bt = BluetoothAudio::get().unwrap();
            loop {
                let ev = sub.recv().await.unwrap();

                match ev {
                    AppEvent::ButtonPress(team) => match team {
                        Team::Blue => {
                            log::info!("Blue team pressed");
                            bt.play_audio(PLUH_SOUND);
                        }
                        Team::Red => {
                            log::info!("Red team pressed");
                            bt.play_audio(LOW_HONOR_SOUND);
                        }
                    },
                }
            }
        }));
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AppState {
    Setup,
    Idle,
    InGame
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
