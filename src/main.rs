use std::{fmt::Display, sync::{Arc, Mutex, RwLock, atomic::AtomicBool}};

use anyhow::{Ok, Result};
use esp_idf_svc::{
    bt::{BdAddr, BtClassic, BtDriver, BtMode, gap::{EspGap, InqMode}},
    hal::{delay::FreeRtos, peripheral::PeripheralRef, prelude::Peripherals},
    nvs::EspDefaultNvsPartition,
};

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let bt = Bluetooth::new()?;

    loop {
        bt.start_discovery()?;
        log::info!("Started discovery");

        FreeRtos::delay_ms(20_000);

        bt.stop_discovery()?;
        log::info!("Stopped discovery");

        for device in bt.discovered_devices().read().unwrap().iter() {
            log::info!("{device}");
        }
    }
}

type BtClassicDriver = BtDriver<'static, BtClassic>;
type EspBtClassicGap = EspGap<'static, BtClassic, Arc<BtClassicDriver>>;

#[derive(Debug, Clone)]
struct BtDevice {
    name: Arc<String>,
    addr: BdAddr
}

impl Display for BtDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}", self.name, self.addr.to_string())
    }
}

struct Bluetooth {
    driver: Arc<BtClassicDriver>,
    connection: Option<BdAddr>,
    gap: EspBtClassicGap,
    discovered_devices: Arc<RwLock<Vec<BtDevice>>>,
    is_in_discovery: AtomicBool
}

impl Bluetooth {
    pub fn new() -> Result<Self> {
        let nvs = EspDefaultNvsPartition::take()?;
        let modem = Peripherals::take()?.modem;
        let driver = Arc::new(BtDriver::new(modem, Some(nvs))?);
        driver.set_device_name("Esp32-dominacao")?;
        let gap = EspGap::new(driver.clone())?;
        Ok(Self {
            connection: None,
            gap,
            driver: driver.clone(),
            discovered_devices: Arc::new(RwLock::new(vec![])),
            is_in_discovery: false.into()
        })
    }

    pub fn discovered_devices(&self) -> Arc<RwLock<Vec<BtDevice>>> {
        self.discovered_devices.clone()
    }

    pub fn start_discovery(&self) -> Result<()> {
        if self.is_in_discovery.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }

        self.gap.start_discovery(InqMode::General, 8, 10)?;

        self.is_in_discovery.store(true, std::sync::atomic::Ordering::Relaxed);

        let devices = self.discovered_devices.clone();
        self.gap.subscribe(move |event| {
            match event {
                esp_idf_svc::bt::gap::GapEvent::DeviceDiscovered { bd_addr, props } => {
                    for prop in props {
                        let p = prop.prop();
                        let mut device_name = None;
                        let addr = bd_addr;
                        match p {
                            esp_idf_svc::bt::gap::DeviceProp::Eir(eir) => {
                                let name = eir.local_name::<BtClassic, BtClassicDriver>();
                                if let Some(name) = name {
                                    device_name = Some(name.to_string());
                                }
                            },
                            _ => {}
                        }

                        let device = BtDevice { name: Arc::new(device_name.unwrap_or("Unknown".to_string())), addr };
                        let mut devices = devices.write().expect("Poisoned");
                        devices.push(device);
                        drop(devices);
                    }
                },
                _ => {}
            }
        })?;

        Ok(())
    }

    pub fn stop_discovery(&self) -> Result<()> {
        if !self.is_in_discovery.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }

        self.gap.stop_discovery()?;
        self.gap.unsubscribe()?;

        Ok(())
    }

    pub fn connect(&mut self, addr: BdAddr) {
    }
}
