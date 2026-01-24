use anyhow::{Ok, Result};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::prelude::Peripherals,
    mdns::EspMdns,
    nvs::EspDefaultNvsPartition,
    timer::EspTaskTimerService,
    wifi::{AsyncWifi, EspWifi},
};

use crate::hardware::bt::BluetoothAudio;
use crate::{
    app::{App, Team},
    hardware::{buttons::InputButton, wifi::Wifi},
    infra::server::{load_web, HttpServer},
};

pub mod app;
pub mod assets;
pub mod hardware;
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
    let mut wifi = Wifi::init(async_wifi);
    let bt = BluetoothAudio::init(bt_modem, Some(nvs.clone()))?;
    let app = App::init(wifi, bt);

    let mut mdns = EspMdns::take()?;
    mdns.set_hostname("dominacao")?;
    mdns.add_service(Some("Sandi Dominacao"), "_http", "_tcp", 80, &[])?;

    let mut server = HttpServer::new();

    register_routes(&mut server);

    core::mem::forget(mdns);
    core::mem::forget(server);

    std::thread::spawn(|| {
        app.run(move |app| {
            if red_btn.is_pressed() {
                app.team_press(Team::Red);
            }

            if blue_btn.is_pressed() {
                app.team_press(Team::Blue);
            }
        });
    });

    Ok(())
}

fn register_routes(server: &mut HttpServer) {
    load_web(server);
}
