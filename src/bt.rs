use std::result::Result::Ok;
use std::{
    fmt::Display,
    sync::{
        atomic::AtomicBool,
        mpsc::{Receiver, Sender},
        Arc, OnceLock, RwLock,
    },
};

use anyhow::Result;
use esp_idf_svc::{
    bt::{
        a2dp::{A2dpEvent, ConnectionStatus, EspA2dp, Source},
        avrc::controller::{AvrccEvent, EspAvrcc},
        gap::{EspGap, InqMode},
        BdAddr, BtClassic, BtDriver,
    },
    hal::{delay::FreeRtos, modem::BluetoothModemPeripheral, peripheral::Peripheral},
    nvs::EspDefaultNvsPartition,
    sys::{
        esp_a2d_media_ctrl, esp_a2d_media_ctrl_t_ESP_A2D_MEDIA_CTRL_START, vRingbufferReturnItem,
        xRingbufferCreate, xRingbufferReceiveUpTo, xRingbufferSend, RingbufHandle_t,
        RingbufferType_t_RINGBUF_TYPE_BYTEBUF,
    },
};

type BtClassicDriver = BtDriver<'static, BtClassic>;
type EspBtClassicGap = EspGap<'static, BtClassic, Arc<BtClassicDriver>>;

enum AudioCommand {
    Play(&'static [u8]), // or file path
    Stop,
}

use std::sync::atomic::{AtomicU32, Ordering};

static AUDIO_GEN: AtomicU32 = AtomicU32::new(0);

fn spawn_audio_task(rx: Receiver<AudioCommand>) {
    std::thread::spawn(move || {
        let bt = BluetoothAudio::get().unwrap();

        const CHUNK: usize = 512;
        const PREFILL: usize = 4096;

        loop {
            match rx.recv() {
                Ok(AudioCommand::Play(data)) => {
                    let my_gen = AUDIO_GEN.load(Ordering::SeqCst);
                    // Hard cut: flush anything pending
                    bt.flush_ringbuffer();

                    // ---- PREFILL ----
                    let prefill = PREFILL.min(data.len());
                    bt.send_bytes(&data[..prefill], esp_idf_svc::sys::TickType_t::MAX);

                    let mut offset = prefill;

                    // ---- STREAM ----
                    while offset < data.len() {
                        // If a newer Play() happened → exit immediately
                        if AUDIO_GEN.load(Ordering::Relaxed) != my_gen {
                            break;
                        }

                        let end = (offset + CHUNK).min(data.len());

                        bt.send_bytes(&data[offset..end], esp_idf_svc::sys::TickType_t::MAX);

                        offset = end;

                        // Small delay to avoid BT starvation
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                }

                Ok(AudioCommand::Stop) => {
                    AUDIO_GEN.fetch_add(1, Ordering::SeqCst);
                    bt.flush_ringbuffer();
                }

                Err(_) => break,
            }
        }
    });
}

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

#[derive(Copy, Clone)]
struct Ringbuf(RingbufHandle_t);

// ESP-IDF ring buffers are thread-safe by design
unsafe impl Send for Ringbuf {}
unsafe impl Sync for Ringbuf {}

static BLUETOOTH_AUDIO: OnceLock<BluetoothAudio> = OnceLock::new();

#[allow(dead_code)]
pub struct BluetoothAudio {
    driver: Arc<BtClassicDriver>,
    connection: RwLock<Option<BtDevice>>,
    gap: EspBtClassicGap,
    discovered_devices: Arc<RwLock<Vec<BtDevice>>>,
    is_in_discovery: AtomicBool,
    a2dp: EspA2dp<'static, BtClassic, Arc<BtClassicDriver>, Source>,
    avrc: Arc<EspAvrcc<'static, BtClassic, Arc<BtClassicDriver>>>,
    ring_buf: Arc<Ringbuf>,
    audio_cmd_tx: Sender<AudioCommand>,
}

impl BluetoothAudio {
    pub fn init<B: BluetoothModemPeripheral>(
        modem: impl Peripheral<P = B> + 'static,
        nvs: Option<EspDefaultNvsPartition>,
    ) -> anyhow::Result<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        let bt = BluetoothAudio::new(modem, nvs, tx)?;
        BLUETOOTH_AUDIO
            .set(bt)
            .map_err(|_| anyhow::anyhow!("Bluetooth already initialized"))?;
        log::info!("Init Bluetooth Audio");
        spawn_audio_task(rx);
        Ok(())
    }

    pub fn get() -> anyhow::Result<&'static Self> {
        let ret = BLUETOOTH_AUDIO.get();

        if ret.is_none() {
            return Err(anyhow::anyhow!("Bluetooth not initialized"));
        }

        Ok(ret.unwrap())
    }

    fn new<B: BluetoothModemPeripheral>(
        modem: impl Peripheral<P = B> + 'static,
        nvs: Option<EspDefaultNvsPartition>,
        tx: Sender<AudioCommand>,
    ) -> Result<Self> {
        let driver = Arc::new(BtDriver::new(modem, nvs)?);
        driver.set_device_name("Esp32dominacao")?;
        let gap = EspGap::new(driver.clone())?;
        gap.request_variable_pin()?;
        let handle = unsafe { xRingbufferCreate(64 * 1024, RingbufferType_t_RINGBUF_TYPE_BYTEBUF) };
        let avrc = EspAvrcc::new(driver.clone())?;
        avrc.subscribe(Self::avrc_event_handler)?;
        let a2dp = EspA2dp::new_source(driver.clone())?;
        a2dp.subscribe(Self::a2dp_event_handler)?;

        Ok(Self {
            connection: RwLock::new(None),
            audio_cmd_tx: tx,
            gap,
            driver: driver.clone(),
            discovered_devices: Arc::new(RwLock::new(vec![])),
            is_in_discovery: false.into(),
            a2dp,
            avrc: Arc::new(avrc),
            ring_buf: Arc::new(Ringbuf(handle)),
        })
    }

    fn avrc_event_handler(ev: AvrccEvent) {
        log::info!("{:#?}", ev);
    }

    fn a2dp_event_handler(ev: A2dpEvent) -> usize {
        match ev {
            esp_idf_svc::bt::a2dp::A2dpEvent::ConnectionState {
                bd_addr,
                status,
                disconnect_abnormal: _,
            } => {
                if status == ConnectionStatus::Connected {
                    unsafe { esp_a2d_media_ctrl(esp_a2d_media_ctrl_t_ESP_A2D_MEDIA_CTRL_START) };
                    log::info!("Started media on {bd_addr}");
                }
                1
            }
            esp_idf_svc::bt::a2dp::A2dpEvent::SourceData(buffer) => {
                let bt = Self::get().unwrap();
                let mut copied = 0;

                unsafe {
                    let mut size = 0;
                    let item = xRingbufferReceiveUpTo(
                        bt.ring_buf.0,
                        &mut size,
                        0, // no block
                        buffer.len(),
                    );

                    if !item.is_null() {
                        core::ptr::copy_nonoverlapping(
                            item as *const u8,
                            buffer.as_mut_ptr(),
                            size,
                        );
                        vRingbufferReturnItem(bt.ring_buf.0, item);
                        copied = size;
                    } else {
                        // Ring buffer empty: fill with silence (zeros) to avoid BT stall
                        core::ptr::write_bytes(buffer.as_mut_ptr(), 0, buffer.len());
                        copied = buffer.len();
                    }
                }

                copied
            }
            any => {
                log::info!("{any:?}");
                1
            }
        }
    }

    pub fn send_bytes(&self, pcm: &[u8], tick_wait: u32) {
        unsafe {
            xRingbufferSend(
                self.ring_buf.0,
                pcm.as_ptr() as *const _,
                pcm.len(),
                tick_wait,
            );
        }
    }
    fn flush_ringbuffer(&self) {
        unsafe {
            let mut size = 0;
            loop {
                let item = xRingbufferReceiveUpTo(self.ring_buf.0, &mut size, 0, usize::MAX);
                if item.is_null() {
                    break;
                }
                vRingbufferReturnItem(self.ring_buf.0, item);
            }
        }
    }

    pub fn play_audio(&self, data: &'static [u8]) {
        AUDIO_GEN.fetch_add(1, Ordering::SeqCst);
        self.audio_cmd_tx.send(AudioCommand::Play(data)).ok();
    }

    pub fn a2dp_connect(&self, device: &BtDevice) -> Result<()> {
        let mut conn = self.connection.write().unwrap();

        if conn.is_some() {
            return Err(anyhow::anyhow!("Already connected"));
        }

        let addr = device.addr.clone();

        *conn = Some(device.clone());

        self.a2dp.connect_source(&addr)?;

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

    pub async fn discover_devices(&self) -> Arc<[BtDevice]> {
        let _ = self.start_discovery();

        FreeRtos::delay_ms(10_000);

        let _ = self.stop_discovery();

        let devices = self.discovered_devices();
        let devices_vec = devices.read().unwrap().clone();
        devices_vec.into()
    }
}
