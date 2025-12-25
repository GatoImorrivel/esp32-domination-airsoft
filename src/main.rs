use std::ffi::CStr;

use anyhow::{Ok, Result};
use esp_idf_svc::{
    eventloop::{
        EspEvent, EspEventDeserializer, EspEventPostData, EspEventSerializer, EspEventSource,
        EspSystemEventLoop,
    },
    hal::delay::FreeRtos,
    sys::EspError,
};

pub mod bt;

pub use bt::Bluetooth;

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let mut bt = Bluetooth::new()?;
    bt.start_discovery()?;
    log::info!("Started discovery");

    FreeRtos::delay_ms(20_000);

    bt.stop_discovery()?;
    log::info!("Stopped discovery");

    let devices = bt.discovered_devices();
    let devices = devices.read().unwrap();
    let device = devices.first().unwrap();

    bt.a2dp_connect(device)?;

    run()?;

    Ok(())
}

fn run() -> Result<(), EspError> {
    let sys_loop = EspSystemEventLoop::take()?;

    let _sub = sys_loop.subscribe::<CustomEvent, _>(|ev| {
        log::info!("{ev:?}");
    });

    esp_idf_svc::hal::task::block_on(core::pin::pin!(async move {
        let mut sub = sys_loop.subscribe_async::<CustomEvent>()?;

        loop {
            let ev = sub.recv().await?;
            log::info!("{ev:?}");
        }
    }))
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug)]
enum CustomEvent {
    Start,
    Tick(u32),
}
unsafe impl EspEventSource for CustomEvent {
    #[allow(clippy::manual_c_str_literals)]
    fn source() -> Option<&'static CStr> {
        // String should be unique across the whole project and ESP IDF
        Some(CStr::from_bytes_with_nul(b"DEMO-SERVICE\0").unwrap())
    }
}

impl EspEventSerializer for CustomEvent {
    type Data<'a> = CustomEvent;

    fn serialize<F, R>(event: &Self::Data<'_>, f: F) -> R
    where
        F: FnOnce(&EspEventPostData) -> R,
    {
        f(&unsafe { EspEventPostData::new(Self::source().unwrap(), Self::event_id(), event) })
    }
}

impl EspEventDeserializer for CustomEvent {
    type Data<'a> = CustomEvent;

    fn deserialize<'a>(data: &EspEvent<'a>) -> Self::Data<'a> {
        *unsafe { data.as_payload::<CustomEvent>() }
    }
}
