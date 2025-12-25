use std::{
    fmt::Display,
    sync::{atomic::AtomicBool, Arc, RwLock},
};

use anyhow::Result;
use esp_idf_svc::{
    bt::{
        a2dp::{ConnectionStatus, EspA2dp, Source},
        avrc::controller::EspAvrcc,
        gap::{EspGap, InqMode},
        BdAddr, BtClassic, BtDriver,
    },
    hal::prelude::Peripherals,
    nvs::EspDefaultNvsPartition,
    sys::{esp_a2d_media_ctrl, esp_a2d_media_ctrl_t_ESP_A2D_MEDIA_CTRL_START},
};

type BtClassicDriver = BtDriver<'static, BtClassic>;
type EspBtClassicGap = EspGap<'static, BtClassic, Arc<BtClassicDriver>>;

#[derive(Debug, Clone)]
pub struct BtDevice {
    name: Option<Arc<String>>,
    addr: BdAddr,
}

impl Display for BtDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name: &str = {
            if let Some(name) = &self.name {
                name.as_str()
            } else {
                "Unknown"
            }
        };
        write!(f, "{} at {}", name, self.addr.to_string())
    }
}

impl PartialEq for BtDevice {
    fn eq(&self, other: &Self) -> bool {
        self.addr == other.addr
    }
}

impl Eq for BtDevice {}

pub struct Bluetooth {
    driver: Arc<BtClassicDriver>,
    connection: Option<BtDevice>,
    gap: EspBtClassicGap,
    discovered_devices: Arc<RwLock<Vec<BtDevice>>>,
    is_in_discovery: AtomicBool,
    a2dp: Option<EspA2dp<'static, BtClassic, Arc<BtClassicDriver>, Source>>,
    avrc: Option<Arc<EspAvrcc<'static, BtClassic, Arc<BtClassicDriver>>>>,
}

impl Bluetooth {
    pub fn new() -> Result<Self> {
        let nvs = EspDefaultNvsPartition::take()?;
        let modem = Peripherals::take()?.modem;
        let driver = Arc::new(BtDriver::new(modem, Some(nvs))?);
        driver.set_device_name("Esp32dominacao")?;
        let gap = EspGap::new(driver.clone())?;
        gap.request_variable_pin()?;

        Ok(Self {
            connection: None,
            gap,
            driver: driver.clone(),
            discovered_devices: Arc::new(RwLock::new(vec![])),
            is_in_discovery: false.into(),
            a2dp: None,
            avrc: None,
        })
    }

    pub fn a2dp_connect(&mut self, device: &BtDevice) -> Result<()> {
        let avrcc = EspAvrcc::new(self.driver.clone())?;
        self.avrc = Some(Arc::new(avrcc));

        self.avrc.as_ref().unwrap().subscribe(move |ev| {
            log::info!("{:#?}", ev);
        })?;

        let a2dp = EspA2dp::new_source(self.driver.clone())?;
        self.a2dp = Some(a2dp);

        self.a2dp.as_ref().unwrap().subscribe(move |ev| {
            match ev {
                esp_idf_svc::bt::a2dp::A2dpEvent::ConnectionState {
                    bd_addr,
                    status,
                    disconnect_abnormal: _,
                } => {
                    if status == ConnectionStatus::Connected {
                        unsafe {
                            esp_a2d_media_ctrl(esp_a2d_media_ctrl_t_ESP_A2D_MEDIA_CTRL_START)
                        };
                        log::info!("Started media on {bd_addr}");
                    }
                }
                esp_idf_svc::bt::a2dp::A2dpEvent::SourceData(buffer) => {
                    if buffer.is_empty() {
                        return 0;
                    }

                    return buffer.len();
                }
                any => {
                    log::info!("{any:?}");
                }
            }
            1
        })?;

        self.a2dp
            .as_ref()
            .unwrap()
            .connect_source(&self.connection.clone().unwrap().addr)?;

        self.connection = Some(device.clone());

        Ok(())
    }

    pub fn discovered_devices(&self) -> Arc<RwLock<Vec<BtDevice>>> {
        self.discovered_devices.clone()
    }

    pub fn start_discovery(&self) -> Result<()> {
        if self
            .is_in_discovery
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Ok(());
        }

        self.is_in_discovery
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let devices = self.discovered_devices.clone();
        self.gap.subscribe(move |event| match event {
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
                        }
                        _ => {}
                    }

                    let device = {
                        if let Some(name) = device_name {
                            BtDevice {
                                addr,
                                name: Some(Arc::new(name)),
                            }
                        } else {
                            BtDevice { name: None, addr }
                        }
                    };
                    let mut devices = devices.write().expect("Poisoned");

                    if !devices.contains(&device) {
                        devices.push(device);
                    } else {
                        let (i, other_device) = devices
                            .iter()
                            .enumerate()
                            .find(|(_, d)| **d == device)
                            .unwrap();

                        if other_device.name.is_none() {
                            devices[i] = device;
                        }
                    }
                    drop(devices);
                }
            }
            _ => {}
        })?;

        self.gap.start_discovery(InqMode::General, 8, 10)?;

        Ok(())
    }

    pub fn stop_discovery(&self) -> Result<()> {
        if !self
            .is_in_discovery
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Ok(());
        }

        self.gap.stop_discovery()?;
        self.gap.unsubscribe()?;

        Ok(())
    }
}
