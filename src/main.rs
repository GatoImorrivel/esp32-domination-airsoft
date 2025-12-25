use std::{
    fmt::Display,
    sync::{atomic::{AtomicBool, AtomicUsize}, Arc, Mutex, RwLock},
    thread,
};

use rand::Rng;

use anyhow::{Ok, Result};
use esp_idf_svc::{
    bt::{
        BdAddr, BtClassic, BtDriver, BtMode, a2dp::{ConnectionStatus, EspA2dp, Source}, avrc::controller::EspAvrcc, gap::{EspGap, InqMode}
    },
    hal::{delay::FreeRtos, peripheral::PeripheralRef, prelude::Peripherals},
    nvs::EspDefaultNvsPartition, sys::{esp_a2d_audio_state_t_ESP_A2D_AUDIO_STATE_STARTED, esp_a2d_media_ctrl, esp_a2d_media_ctrl_t_ESP_A2D_MEDIA_CTRL_START, esp_bt_pin_type_t_ESP_BT_PIN_TYPE_VARIABLE},
};

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

    // Spawn a background task to keep the system alive
    thread::spawn(|| {
        loop {
            FreeRtos::delay_ms(1000);
        }
    });

    // Main thread can now return or do other things
    loop {
        FreeRtos::delay_ms(5000);
    }
}

type BtClassicDriver = BtDriver<'static, BtClassic>;
type EspBtClassicGap = EspGap<'static, BtClassic, Arc<BtClassicDriver>>;

#[derive(Debug, Clone)]
struct BtDevice {
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

struct Bluetooth {
    driver: Arc<BtClassicDriver>,
    connection: Option<BtDevice>,
    gap: EspBtClassicGap,
    discovered_devices: Arc<RwLock<Vec<BtDevice>>>,
    is_in_discovery: AtomicBool,
    a2dp: Option<EspA2dp<'static, BtClassic, Arc<BtClassicDriver>, Source>>,
    avrc: Option<Arc<EspAvrcc<'static, BtClassic, Arc<BtClassicDriver>>>>,
    sample_index: Arc<AtomicUsize>,
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
            sample_index: Arc::new(AtomicUsize::new(0)),
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

        self.connection = Some(device.clone());

        let sample_index = self.sample_index.clone();
        self.a2dp.as_ref().unwrap().subscribe(move |ev| {
            match ev {
                esp_idf_svc::bt::a2dp::A2dpEvent::ConnectionState { bd_addr, status, disconnect_abnormal } => {
                    if status == ConnectionStatus::Connected {
                        unsafe {esp_a2d_media_ctrl(esp_a2d_media_ctrl_t_ESP_A2D_MEDIA_CTRL_START)};
                    }
                },
                esp_idf_svc::bt::a2dp::A2dpEvent::SourceData(buffer) => {
                    if buffer.is_empty() {
                        return 0;
                    }

                    // Sine wave parameters
                    const SAMPLE_RATE: f32 = 44100.0;
                    const FREQUENCY: f32 = 440.0; // A4 note
                    const AMPLITUDE: f32 = 20000.0; // Louder amplitude

                    let samples = buffer.len() / 2;

                    let mut idx = sample_index.load(std::sync::atomic::Ordering::Relaxed);
                    for i in 0..samples {
                        let phase = 2.0 * std::f32::consts::PI * FREQUENCY * (idx as f32) / SAMPLE_RATE;
                        let sample: i16 = (AMPLITUDE * phase.sin()) as i16;

                        let bytes = sample.to_le_bytes();
                        buffer[i * 2] = bytes[0];
                        buffer[i * 2 + 1] = bytes[1];

                        idx += 1;
                    }

                    sample_index.store(idx, std::sync::atomic::Ordering::Relaxed);

                    return buffer.len() as usize;
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