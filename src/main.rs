use std::{ffi::CStr, sync::Arc};

use anyhow::{Ok, Result};
use esp_idf_svc::{
    eventloop::{
        EspEvent, EspEventDeserializer, EspEventPostData, EspEventSerializer, EspEventSource,
        EspSystemEventLoop,
    },
    hal::{delay::FreeRtos, prelude::Peripherals},
    nvs::EspDefaultNvsPartition,
};

use crate::bt::BluetoothAudio;
use crate::buttons::InputButton;

pub mod bt;
pub mod buttons;

const PLUH_SOUND: &[u8; 142764] = include_bytes!("../data/pluh.raw");
const LOW_HONOR_SOUND: &[u8; 918692] = include_bytes!("../data/low-honor-rdr-2.raw");

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    BluetoothAudio::init(peripherals.modem, Some(nvs))?;

    let mut red_btn = InputButton::new(peripherals.pins.gpio19, 50)?;
    let mut blue_btn = InputButton::new(peripherals.pins.gpio18, 50)?;

    let sys_loop = Arc::new(EspSystemEventLoop::take()?);

    let red_pub = sys_loop.clone();
    red_btn.set_callback(move || {
        red_pub
            .post::<AppEvent>(&AppEvent::ButtonPress(Team::Red), 0)
            .unwrap();
    })?;

    let blue_pub = sys_loop.clone();
    blue_btn.set_callback(move || {
        blue_pub
            .post::<AppEvent>(&AppEvent::ButtonPress(Team::Blue), 0)
            .unwrap();
    })?;

    let bt = BluetoothAudio::get()?;
    bt.start_discovery()?;
    log::info!("Started discovery");

    FreeRtos::delay_ms(20_000);

    bt.stop_discovery()?;
    log::info!("Stopped discovery");

    let devices = bt.discovered_devices();
    let devices = devices.read().unwrap();
    let device = devices.first().unwrap();

    bt.a2dp_connect(device)?;

    esp_idf_svc::hal::task::block_on(core::pin::pin!(async move {
        let mut sub = sys_loop.subscribe_async::<AppEvent>().unwrap();
        let bt = BluetoothAudio::get().unwrap();
        loop {
            let ev = sub.recv().await.unwrap();

            match ev {
                AppEvent::ButtonPress(team) => match team {
                    Team::Blue =>  {
                        log::info!("Blue team pressed");
                        bt.play_audio(PLUH_SOUND);
                    },
                    Team::Red => {
                        log::info!("Red team pressed");
                        bt.play_audio(LOW_HONOR_SOUND);
                    }
                },
            }
        }
    }));

    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum Team {
    Red,
    Blue,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug)]
enum AppEvent {
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
