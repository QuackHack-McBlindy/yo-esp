#![no_std]

use defmt::{info, error};
use defmt::Debug2Format;
use esp_hal::i2s::master::{I2sRx, I2sTx, asynch::{I2sReadDmaTransferAsync, I2sWriteDmaTransferAsync}};
use esp_hal::Async;
use alloc::vec::Vec;
use alloc::vec;
use alloc::boxed::Box;
use embassy_net::{Stack, tcp::TcpSocket, IpAddress};
use embassy_time::{Duration, Timer, Instant};
use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pipe::Pipe;
use core::net::SocketAddr;
use core::future::Future;
use core::pin::Pin;

extern crate alloc;

const STEREO_SAMPLES_PER_READ: usize = 256;
const MONO_SAMPLES_PER_READ: usize = STEREO_SAMPLES_PER_READ / 2;
/// MUST MATCH WAKE WORD CHUNK SIZE
pub const OWW_MODEL_CHUNK_SIZE: usize = 1280;
const DEBUG_MIC: bool = false;

const TCP_RX_BUF_SIZE: usize = 1024;
const TCP_TX_BUF_SIZE: usize = 4096;

pub const SPEAKER_DMA_BUFFER_SIZE: usize = 16368;

const STEREO_SAMPLES_PER_WRITE: usize = 256;
const PLAYBACK_TCP_RX_BUF_SIZE: usize = 4096;
const PLAYBACK_TCP_TX_BUF_SIZE: usize = 2048;
const RING_BUFFER_SIZE: usize = 16384;

const DING_SOUND: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sound/ding_esp.raw"));
const DONE_SOUND: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sound/done_esp.wav"));
const FAIL_SOUND: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sound/fail_esp.wav"));


pub struct Microphone {
    i2s_rx: I2sRx<'static, Async>,
    stereo_buffer: [u8; STEREO_SAMPLES_PER_READ * 2],
    mono_i16: [i16; MONO_SAMPLES_PER_READ],
    mono_f32: [f32; MONO_SAMPLES_PER_READ],
    accum_buffer: Vec<f32>,
    silent: bool,
}

impl Microphone {
    pub fn new(i2s_rx: I2sRx<'static, Async>) -> Self {
        Self {
            i2s_rx,
            stereo_buffer: [0u8; STEREO_SAMPLES_PER_READ * 2],
            mono_i16: [0i16; MONO_SAMPLES_PER_READ],
            mono_f32: [0f32; MONO_SAMPLES_PER_READ],
            accum_buffer: Vec::with_capacity(OWW_MODEL_CHUNK_SIZE),
            silent: false,
        }
    }


    pub async fn read_chunk(&mut self) -> Result<(Vec<f32>, bool), ()> {
        while self.accum_buffer.len() < OWW_MODEL_CHUNK_SIZE {
            match self.i2s_rx.read_dma_async(&mut self.stereo_buffer).await {
                Ok(()) => {}
                Err(e) => {
                    defmt::error!("I2S read_dma_async failed: {}", Debug2Format(&e));
                    return Err(());
                }
            }

            if DEBUG_MIC {
                let stereo = unsafe {
                    core::slice::from_raw_parts(
                        self.stereo_buffer.as_ptr() as *const i16,
                        STEREO_SAMPLES_PER_READ,
                    )
                };
                info!("[MIC i16]: {:?}", &stereo[..8.min(stereo.len())]);
            }

            let stereo = unsafe {
                core::slice::from_raw_parts(
                    self.stereo_buffer.as_ptr() as *const i16,
                    STEREO_SAMPLES_PER_READ,
                )
            };

            for (i, chunk) in stereo.chunks(2).enumerate() {
                self.mono_i16[i] = ((chunk[0] as i32 + chunk[1] as i32) / 2) as i16;
            }

            for (i, &s) in self.mono_i16.iter().enumerate() {
                self.mono_f32[i] = s as f32 / 32768.0;
            }
            self.accum_buffer.extend_from_slice(&self.mono_f32[..MONO_SAMPLES_PER_READ]);
        }

        let chunk: Vec<f32> = self.accum_buffer.drain(..OWW_MODEL_CHUNK_SIZE).collect();

        let all_zero = chunk.iter().all(|&s| s == 0.0);
        if all_zero {
            if !self.silent {
                info!("🎙️⚠️ Mic zero zero zero!");
                self.silent = true;
            }
        } else if self.silent {
            info!("🎙️✅ Mic OK!");
            self.silent = false;
        }
        Ok((chunk, all_zero))
    }
}



pub trait CommandHandler {
    fn on_detected(&mut self) -> Pin<Box<dyn Future<Output = ()> + '_>>;
    fn on_thinking(&mut self) -> Pin<Box<dyn Future<Output = ()> + '_>>;
    fn on_executed(&mut self, elapsed_ms: Option<u64>) -> Pin<Box<dyn Future<Output = ()> + '_>>;
    fn on_failed(&mut self, elapsed_ms: Option<u64>) -> Pin<Box<dyn Future<Output = ()> + '_>>;
}




#[embassy_executor::task]
pub async fn audio_capture_task(
    i2s_rx: I2sRx<'static, Async>,
    stack: &'static Stack<'static>,
    remote_addr: SocketAddr,
    room: &'static str,
    handler: alloc::boxed::Box<dyn CommandHandler>,
) {
    let remote_endpoint = match remote_addr {
        SocketAddr::V4(v4) => (IpAddress::Ipv4(v4.ip().octets().into()), v4.port()),
        SocketAddr::V6(_) => {
            error!("IPv6 not supported");
            return;
        }
    };

    stack.wait_link_up().await;
    stack.wait_config_up().await;

    let mut mic = Microphone::new(i2s_rx);
    let room_bytes = room.as_bytes();
    let room_len = room_bytes.len() as u32;

    loop {
        let mut rx_buffer = [0u8; TCP_RX_BUF_SIZE];
        let mut tx_buffer = [0u8; TCP_TX_BUF_SIZE];
        let mut socket = TcpSocket::new(stack.clone(), &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));

        if let Err(e) = socket.connect(remote_endpoint).await {
            error!("❌ connect error: {:?}, retrying in 15s", e);
            Timer::after(Duration::from_secs(15)).await;
            continue;
        }
        info!("📡 ☑️ 🎙️ to {}", remote_addr);

        let mut handshake_ok = true;
        let len_bytes = room_len.to_le_bytes();
        let mut written = 0;
        while written < len_bytes.len() {
            match socket.write(&len_bytes[written..]).await {
                Ok(n) => written += n,
                Err(e) => {
                    error!("handshake length fail: {:?}", e);
                    handshake_ok = false;
                    break;
                }
            }
        }
        if handshake_ok && room_len > 0 {
            let mut written = 0;
            while written < room_bytes.len() {
                match socket.write(&room_bytes[written..]).await {
                    Ok(n) => written += n,
                    Err(e) => {
                        error!("failed to send room name: {:?}", e);
                        handshake_ok = false;
                        break;
                    }
                }
            }
        }
        if let Err(e) = socket.flush().await {
            error!("failed to flush handshake: {:?}", e);
            handshake_ok = false;
        }
        if !handshake_ok {
            let _ = socket.close();
            Timer::after(Duration::from_secs(15)).await;
            continue;
        }

        'stream: loop {
            let mut command_start: Option<Instant> = None;

            let (chunk, _silent): (Vec<f32>, bool) = match mic.read_chunk().await {
                Ok(pair) => pair,
                Err(_) => {
                    error!("I2S read ERROR");
                    Timer::after(Duration::from_millis(10)).await;
                    continue;
                }
            };

            let mut chunk_buffer = vec![0u8; 4 + OWW_MODEL_CHUNK_SIZE * 4];
            chunk_buffer[0..4].copy_from_slice(&(OWW_MODEL_CHUNK_SIZE as u32).to_le_bytes());
            for (i, &sample) in chunk.iter().enumerate() {
                let offset = 4 + i * 4;
                chunk_buffer[offset..offset + 4].copy_from_slice(&sample.to_le_bytes());
            }

            let mut written = 0;
            while written < chunk_buffer.len() {
                match socket.write(&chunk_buffer[written..]).await {
                    Ok(n) => written += n,
                    Err(e) => {
                        error!("failed to send audio chunk: {:?}", e);
                        break 'stream;
                    }
                }
            }
            if let Err(e) = socket.flush().await {
                error!("Failed to flush! {:?}", e);
                break 'stream;
            }

            let mut byte_buf = [0u8; 1];
            let read_fut = socket.read(&mut byte_buf);
            let timeout_fut = Timer::after(Duration::from_millis(10));
            match select(read_fut, timeout_fut).await {
                embassy_futures::select::Either::First(Ok(1)) => {
                    match byte_buf[0] {
                        0x01 => {
                            info!("💥 DETECTED Wake Word!");
                            handler.on_detected().await;
                        }
                        0x02 => {
                            info!("🧠 THINKING...");
                            command_start = Some(Instant::now());
                            handler.on_thinking().await;
                        }
                        0x03 => {
                            let elapsed = command_start.map(|s| s.elapsed().as_millis());
                            if let Some(ms) = elapsed {
                                info!("✅ Executed command! Took {} ms", ms);
                            } else {
                                info!("✅ Executed command!");
                            }
                            handler.on_executed(elapsed).await;
                            command_start = None;
                        }
                        0x04 => {
                            let elapsed = command_start.map(|s| s.elapsed().as_millis());
                            if let Some(ms) = elapsed {
                                info!("💩 FAILED execution ({} ms)", ms);
                            } else {
                                info!("💩 FAILED execution!");
                            }
                            handler.on_failed(elapsed).await;
                            command_start = None;
                        }
                        _ => info!("Unexpected byte: 0x{:02x}", byte_buf[0]),
                    }
                }
                embassy_futures::select::Either::First(Ok(_)) => {}
                embassy_futures::select::Either::First(Err(e)) => {
                    error!("socket read error: {:?}", e);
                    break 'stream;
                }
                embassy_futures::select::Either::Second(_) => {}
            }
        }

        info!("❌ reconnecting...");
        let _ = socket.close();
        Timer::after(Duration::from_secs(15)).await;
    }
}

static PIPE: Pipe<CriticalSectionRawMutex, RING_BUFFER_SIZE> = Pipe::new();


pub fn play(data: &[u8]) -> usize {
    PIPE.try_write(data).unwrap_or(0)
}


pub async fn play_sound(sound: &'static [u8]) {
    let mut offset = 0;
    while offset < sound.len() {
        let written = play(&sound[offset..]);
        if written == 0 {
            Timer::after(Duration::from_millis(1)).await;
        } else {
            offset += written;
        }
    }
}


pub async fn play_ding() {
    play_sound(DING_SOUND).await;
}

pub async fn play_done() {
    play_sound(DONE_SOUND).await;
}

pub async fn play_fail() {
    play_sound(FAIL_SOUND).await;
}


#[embassy_executor::task]
pub async fn speaker_task(
    mut transfer: I2sWriteDmaTransferAsync<'static, &'static mut [u8; SPEAKER_DMA_BUFFER_SIZE]>,
) -> ! {
    let mut pipe_buf = [0u8; 1024];
    let silence = [0u8; 256];

    loop {
        let free = transfer.available().await.unwrap();
        if free == 0 {
            Timer::after(Duration::from_micros(100)).await;
            continue;
        }

        let to_read = free.min(pipe_buf.len());
        let read_future = PIPE.read(&mut pipe_buf[..to_read]);
        let timeout = Timer::after(Duration::from_millis(2));

        match select(read_future, timeout).await {
            Either::First(n) if n > 0 => {
                let _ = transfer.push(&pipe_buf[..n]).await;
            }
            _ => {
                let mut remaining = free;
                while remaining > 0 {
                    let chunk = remaining.min(silence.len());
                    let _ = transfer.push(&silence[..chunk]).await;
                    remaining -= chunk;
                }
            }
        }
    }
}


#[embassy_executor::task]
pub async fn stream_speaker(
    stack: &'static Stack<'static>,
    listen_port: u16,
) {
    stack.wait_link_up().await;
    stack.wait_config_up().await;
    info!("📡 ☑️ 🔊 Listening on port {}", listen_port);

    loop {
        let mut rx_buffer = [0u8; PLAYBACK_TCP_RX_BUF_SIZE];
        let mut tx_buffer = [0u8; PLAYBACK_TCP_TX_BUF_SIZE];
        let mut socket = TcpSocket::new(stack.clone(), &mut rx_buffer, &mut tx_buffer);

        if let Err(e) = socket.accept(listen_port).await {
            error!("accept error: {:?}", e);
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }

        info!("audio client connected");
        socket.set_timeout(Some(Duration::from_secs(30)));

        let mut buf = [0u8; 1024];
        loop {
            match socket.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let mut written = 0;
                    while written < n {
                        let w = play(&buf[written..n]);
                        if w == 0 {
                            Timer::after(Duration::from_micros(500)).await;
                        } else {
                            written += w;
                        }
                    }
                }
                Err(e) => {
                    error!("read error: {:?}", e);
                    break;
                }
            }
        }
        info!("client disconnected");
        let _ = socket.close();
    }
}
