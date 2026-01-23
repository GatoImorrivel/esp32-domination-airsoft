use anyhow::{Ok, Result};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop, hal::prelude::Peripherals, nvs::EspDefaultNvsPartition, sys::l64a, timer::EspTaskTimerService, wifi::{AsyncWifi, EspWifi}
};

use crate::{app::{App, AppClient, Team}, hardware::{buttons::InputButton, wifi::Wifi}, infra::server::{HttpServer, load_svelte}};
use crate::{
    hardware::bt::BluetoothAudio,
};

pub mod assets;
pub mod hardware;
pub mod app;
mod infra;

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let (wifi_modem, bt_modem) = peripherals.modem.split();

    let wifi_timer = EspTaskTimerService::new()?;
    let async_wifi = AsyncWifi::wrap(
        EspWifi::new(wifi_modem, sys_loop.clone(), Some(nvs.clone()))?,
        sys_loop.clone(),
        wifi_timer,
    )?;

    let red_btn = InputButton::new(peripherals.pins.gpio19, 50)?;
    let blue_btn = InputButton::new(peripherals.pins.gpio18, 50)?;
    let wifi = Wifi::init(async_wifi);
    let bt = BluetoothAudio::init(bt_modem, Some(nvs.clone()))?;
    let app = App::init(wifi, bt);
    let mut server = HttpServer::new();

    register_routes(&mut server);

    esp_idf_svc::hal::task::block_on(async move {
        app.run(move |client| {
            if red_btn.is_pressed() {
                let result = client.team_press(Team::Red);
                if result.is_err() {
                    log::error!("Failed to register red team press");
                }
            }

            if blue_btn.is_pressed() {
                let result = client.team_press(Team::Blue);
                if result.is_err() {
                    log::error!("Failed to register blue team press");
                }
            }
        }).await;
    });

    Ok(())
}

fn register_routes(server: &mut HttpServer) {
    load_svelte(server);
}
