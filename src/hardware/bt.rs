use std::fmt::Debug;
use std::result::Result::Ok;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use std::{
    fmt::Display,
    sync::{
        atomic::AtomicBool,
        mpsc::{Receiver, Sender},
        RwLock,
    },
};

use anyhow::{anyhow, Result};
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::{
    bt::{
        a2dp::{A2dpEvent, ConnectionStatus, EspA2dp, Source},
        gap::{EspGap, InqMode},
        BdAddr, BtClassic, BtDriver,
    },
    hal::{modem::BluetoothModemPeripheral, peripheral::Peripheral},
    nvs::EspDefaultNvsPartition,
    sys::{
        esp_a2d_media_ctrl, esp_a2d_media_ctrl_t_ESP_A2D_MEDIA_CTRL_START, vRingbufferReturnItem,
        xRingbufferCreate, xRingbufferReceiveUpTo, xRingbufferSend, RingbufHandle_t,
        RingbufferType_t_RINGBUF_TYPE_BYTEBUF,
    },
};
use serde::{de, Deserialize, Serialize};

type BtClassicDriver = BtDriver<'static, BtClassic>;
type EspBtClassicGap = EspGap<'static, BtClassic, Arc<BtClassicDriver>>;

enum AudioCommand {
    Play(&'static [u8]),
    Stop,
}

use std::sync::atomic::{AtomicU32, Ordering};

static AUDIO_GEN: AtomicU32 = AtomicU32::new(0);

fn spawn_audio_task(bt: Arc<BluetoothAudio>, rx: Receiver<AudioCommand>) {
    std::thread::spawn(move || {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BtDevice {
    pub name: Option<String>,
    pub addr: [u8; 6],
}

impl Display for BtDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name: &str = {
            if let Some(name) = &self.name {
                name
            } else {
                "Unknown"
            }
        };
        write!(f, "{} at {}", name, BdAddr::from_bytes(self.addr))
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

#[allow(dead_code)]
pub struct BluetoothAudio {
    driver: Arc<BtClassicDriver>,
    gap: EspBtClassicGap,
    is_in_discovery: AtomicBool,
    a2dp: EspA2dp<'static, BtClassic, Arc<BtClassicDriver>, Source>,
    ring_buf: Arc<Ringbuf>,
    audio_cmd_tx: Sender<AudioCommand>,
}

impl Debug for BluetoothAudio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Bluetooth Audio")
    }
}

impl BluetoothAudio {
    pub fn init<B: BluetoothModemPeripheral>(
        modem: impl Peripheral<P = B> + 'static,
        nvs: Option<EspDefaultNvsPartition>,
    ) -> anyhow::Result<Arc<Self>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let bt = Arc::new(BluetoothAudio::new(modem, nvs, tx)?);
        log::info!("Init Bluetooth Audio");
        spawn_audio_task(bt.clone(), rx);
        let a2dp_bt = bt.clone();
        bt.a2dp
            .subscribe(move |ev| Self::a2dp_event_handler(a2dp_bt.clone(), ev))?;
        Ok(bt.clone())
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
        let a2dp = EspA2dp::new_source(driver.clone())?;

        Ok(Self {
            audio_cmd_tx: tx,
            gap,
            driver: driver.clone(),
            is_in_discovery: false.into(),
            a2dp,
            ring_buf: Arc::new(Ringbuf(handle)),
        })
    }

    fn a2dp_event_handler(bt: Arc<Self>, ev: A2dpEvent) -> usize {
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

    pub fn a2dp_connect(&self, addr: &BdAddr) -> Result<()> {
        self.a2dp.connect_source(addr)?;

        Ok(())
    }

    pub fn discover_devices(
        &self,
        duration: u8,
        max_responses: usize,
    ) -> anyhow::Result<Vec<BtDevice>> {
        let devices: Arc<Mutex<Vec<BtDevice>>> = Arc::new(Mutex::new(vec![]));
        let devices_handler = devices.clone();

        self.gap.subscribe(move |event| match event {
            esp_idf_svc::bt::gap::GapEvent::DeviceDiscovered { bd_addr, props } => {
                let mut devices = devices_handler.lock().unwrap();
                let has_device = devices.iter().find(|d| d.addr == bd_addr.addr()).is_some();
                if has_device {
                    return;
                }
                let mut device_name = None;
                for prop in props {
                    let p = prop.prop();
                    match p {
                        esp_idf_svc::bt::gap::DeviceProp::Eir(eir) => {
                            let name = eir.local_name::<BtClassic, BtClassicDriver>();
                            if let Some(name) = name {
                                device_name = Some(name.to_owned());
                            }
                        }
                        _ => {}
                    }
                }
                let device = BtDevice {
                    name: device_name,
                    addr: bd_addr.addr(),
                };
                log::info!("Discovered {:?}", &device);

                devices.push(device.clone());
            }
            _ => {}
        })?;

        self.gap
            .start_discovery(InqMode::General, duration, max_responses)?;
        FreeRtos::delay_ms(duration as u32 * 1000);
        self.gap.stop_discovery()?;
        self.gap.unsubscribe()?;

        let device = match Arc::try_unwrap(devices) {
            Ok(mutex) => {
                let device = mutex.into_inner().unwrap();
                device
            }
            Err(_) => panic!("still shared"),
        };

        Ok(device)
    }
}
