use anyhow::{Ok, Result};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::prelude::Peripherals,
    nvs::EspDefaultNvsPartition,
    timer::EspTaskTimerService,
    wifi::{AsyncWifi, EspWifi},
};

use crate::{app::App, buttons::InputButton, wifi::Wifi};
use crate::{
    app::{AppEvent, Team},
    bt::BluetoothAudio,
};

pub mod app;
pub mod assets;
pub mod bt;
pub mod buttons;
pub mod wifi;

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let (wifi_modem, bt_modem) = peripherals.modem.split();
    BluetoothAudio::init(bt_modem, Some(nvs.clone()))?;

    let wifi_timer = EspTaskTimerService::new()?;
    let async_wifi = AsyncWifi::wrap(
        EspWifi::new(wifi_modem, sys_loop.clone(), Some(nvs))?,
        sys_loop.clone(),
        wifi_timer,
    )?;
    let wifi = Wifi::new(async_wifi);

    let mut red_btn = InputButton::new(peripherals.pins.gpio19, 50)?;
    let red_pub = sys_loop.clone();
    red_btn.set_callback(move || {
        red_pub
            .post::<AppEvent>(&AppEvent::ButtonPress(Team::Red), 0)
            .unwrap();
    })?;

    let mut blue_btn = InputButton::new(peripherals.pins.gpio18, 50)?;
    let blue_pub = sys_loop.clone();
    blue_btn.set_callback(move || {
        blue_pub
            .post::<AppEvent>(&AppEvent::ButtonPress(Team::Blue), 0)
            .unwrap();
    })?;

    let app = App::default();

    app.run(wifi, sys_loop.clone());

    Ok(())
}
