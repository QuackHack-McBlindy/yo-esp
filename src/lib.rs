// ── YO-ESP https://github.com/QuackHack-McBlindy/yo-esp
// A BARE-METAL BIDIRECTIONAL ESP32 I2S AUDIO STREAMING CRATE
// USE WITH `yo` - https://github.com/QuackHack-McBlindy/yo
// FOR A EXTREMELY FAST, PRIVACY-FIRST OFFLINE VOICE ASSISTANT
// ───────────────────────────────────────────────────────────────────────
// ALL COMMUNICATION IS DONE OVER TCP 
// THIS CRATE REQUIRES THE USER TP HAVE A `embassy-net` STACK ALREADY CONFIGURED
// ───────────────────────────────────────────────────────────────────────
// HOW TO USE THIS CRATE:
// 1. CONSTRUCT THE VOICE HANDLER (EXAMPLE)
// ```
// struct VoiceHandler;

// impl yo_esp::CommandHandler for VoiceHandler {
//     fn on_detected(&mut self) -> core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = ()> + '_>> {
//         alloc::boxed::Box::pin(async {
//             yo_esp::play_ding().await;            
//         })
//     }
//     fn on_thinking(&mut self) -> core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = ()> + '_>> {
//         alloc::boxed::Box::pin(async {
//             // ...
//         })
//     }
//     fn on_executed(&mut self, _ms: Option<u64>) -> core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = ()> + '_>> {
//         alloc::boxed::Box::pin(async move {
//             yo_esp::play_done().await;
//         })
//     }
//     fn on_failed(&mut self, _ms: Option<u64>) -> core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = ()> + '_>> {
//         alloc::boxed::Box::pin(async move {
//             yo_esp::play_fail().await;
//         })
//     }
// }
// ```
// 2. INIT HANDLER IN main(): 
// ```let handler: alloc::boxed::Box<dyn yo_esp::CommandHandler> = alloc::boxed::Box::new(VoiceHandler);```
// 3. START SPEAKER TASK (REQUIRED IF RUNNING I2S FULL-DUPLEX)
// ```spawner.spawn(yo_esp::speaker_task(tx_transfer).unwrap());```
// 4. START THE MICROPHONE TASK:
// ```spawner.spawn(yo_esp::audio_capture_task(i2s_rx, stack, backend_host, backend_port, "esp", handler));```
// 5. OPTIONAL - ACCEPT INCOMING AUDIO TRAFFIC AND PLAY IT ON ESP32 SPEAKER - BY STARTING THE STREAMING TASK:
// ```spawner.spawn(yo_esp::stream_speaker(stack, backend_port));```
// ───────────────────────────────────────────────────────────────────────
// USAGE AFTER TASKS ARE SPAWNED - CALL FROM ANYWHERE:
// START SPEAKER: ```yo_esp::SPEAKER_CMD.send(yo_esp::SpeakerCommand::Start).await;```
// STOP SPEAKER: ```yo_esp::SPEAKER_CMD.send(yo_esp::SpeakerCommand::Stop).await;```
// ENABLE SSTREAMING: ```yo_esp::STREAM_CMD.send(yo_esp::StreamCommand::Start).await;```
// DISABLE STREAMING: ```yo_esp::STREAM_CMD.send(yo_esp::StreamCommand::Stop).await;```
// ENABLE WAKEWORD DETECTION: ```yo_esp::VOICE_CMD.send(yo_esp::VoiceCommand::Enabled).await;```
// DISABLE WAKE-WORD DETECTION: ```yo_esp::VOICE_CMD.send(yo_esp::VoiceCommand::Disabled).await;```
// EXAMPLE OF A PUSH-TO-TALK EMBASSY-EXECUTOR TASK:
// ```
// #[embassy_executor::task]
// pub async fn push_to_talk_task(
//     mut my_button: esp_hal::gpio::Input<'static>,
// ) {
//     loop {
//         let button_held = my_button.is_low();
//         if button_held {
//             let _ = yo_esp::VOICE_CMD.send(yo_esp::VoiceCommand::Pushed).await;
//             while my_button.is_low() {
//                 embassy_time::Timer::after_millis(10).await;
//             }
//             let _ = yo_esp::VOICE_CMD.send(yo_esp::VoiceCommand::Released).await;
//         }
//         embassy_time::Timer::after_millis(50).await;
//     }
// }
// ```
// ───────────────────────────────────────────────────────────────────────

#![no_std]
#![forbid(unsafe_code)]
#[allow(dead_code)]

use defmt::{info, debug, error};
use defmt::Debug2Format;
use esp_hal::i2s::master::{I2sRx, asynch::{I2sWriteDmaTransferAsync}};
use esp_hal::Async;
use alloc::vec::Vec;

use alloc::boxed::Box;
use embassy_net::{Stack, tcp::TcpSocket, IpAddress};
use embassy_time::{Duration, Timer, Instant};
use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pipe::Pipe;
use core::net::SocketAddr;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, Ordering};
use libm::sqrtf;
use defmt::Format;


extern crate alloc;

// ───────────────────────────────────────────────────────────────────────
// CONSTANTS

pub static MEDIA_IS_PLAYING: AtomicBool = AtomicBool::new(false);
const STEREO_SAMPLES_PER_READ: usize = 256;
const _MONO_SAMPLES_PER_READ: usize = STEREO_SAMPLES_PER_READ / 2;
/// MUST MATCH WAKE WORD CHUNK SIZE
pub const OWW_MODEL_CHUNK_SIZE: usize = 1280;

const TCP_RX_BUF_SIZE: usize = 1024;
const TCP_TX_BUF_SIZE: usize = 4096;

pub const SPEAKER_DMA_BUFFER_SIZE: usize = 65472;

const _STEREO_SAMPLES_PER_WRITE: usize = 256;
const PLAYBACK_TCP_RX_BUF_SIZE: usize = 4096;
const PLAYBACK_TCP_TX_BUF_SIZE: usize = 2048;
const RING_BUFFER_SIZE: usize = 16384;

// SOUND FILES (FEATURE ENABLED)
#[cfg(feature = "sounds")]
const DING_SOUND: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sound/ding_esp.raw"));
#[cfg(feature = "sounds")]
const DONE_SOUND: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sound/done_esp.wav"));
#[cfg(feature = "sounds")]
const FAIL_SOUND: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sound/fail_esp.wav"));
const BEEP_SOUND: &[u8] = 
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sound/beep_esp.wav"));

// VOICE COMMAND CONSTRUCTOR
pub trait CommandHandler {
    fn on_detected(&mut self) -> Pin<Box<dyn Future<Output = ()> + '_>>;
    fn on_thinking(&mut self) -> Pin<Box<dyn Future<Output = ()> + '_>>;
    fn on_executed(&mut self, elapsed_ms: Option<u64>) -> Pin<Box<dyn Future<Output = ()> + '_>>;
    fn on_failed(&mut self, elapsed_ms: Option<u64>) -> Pin<Box<dyn Future<Output = ()> + '_>>;
}

// ───────────────────────────────────────────────────────────────────────
// EMBASSY SYNC COMMANDS

// MICROPHONE
#[derive(Format)]
pub enum VoiceCommand {
    Enabled,
    Disabled,
    Pushed,
    Released,
    Intercom,
}

pub static VOICE_CMD: embassy_sync::channel::Channel<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    VoiceCommand,
    3,
> = embassy_sync::channel::Channel::new();

// SPEAKER
#[derive(Format)]
pub enum SpeakerCommand {
    Start,
    Stop,
}

pub static SPEAKER_CMD: embassy_sync::channel::Channel<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    SpeakerCommand,
    1,
> = embassy_sync::channel::Channel::new();

#[derive(Format)]
pub enum StreamCommand {
    Start,
    Stop,
}

pub static STREAM_CMD: embassy_sync::channel::Channel<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    StreamCommand,
    1,
> = embassy_sync::channel::Channel::new();

// ───────────────────────────────────────────────────────────────────────
// HELPERS

// SIMPLE RESAMPLER
fn _linear_resample(input: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
    if input_rate == output_rate {
        return input.to_vec();
    }

    let ratio = input_rate as f64 / output_rate as f64;
    let output_len_f64 = (input.len() as f64) / ratio;
    let output_len = libm::ceil(output_len_f64) as usize;
    let mut output = Vec::with_capacity(output_len);

    let mut input_idx = 0.0f64;
    for _ in 0..output_len {
        let idx_floor = input_idx as usize;
        let idx_ceil = (idx_floor + 1).min(input.len() - 1);
        let frac = input_idx - idx_floor as f64;

        let sample = if idx_floor < input.len() - 1 {
            // LINEAR INTERPOLATION BETWEEN TWO SAMPLES
            let a = input[idx_floor] as f64;
            let b = input[idx_ceil] as f64;
            (a + (b - a) * frac) as f32
        } else {
            // LAST SAMPLE – REPEAT
            input[idx_floor] as f32
        };

        output.push(sample);
        input_idx += ratio;
    }

    output
}

// RMS BASED VAD
fn _rms_f32(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_squares: f32 = samples.iter().map(|&x| x * x).sum();
    sqrtf(sum_squares / samples.len() as f32)
}


pub struct DcBlock {
    x1: f32,
    y1: f32,
    r: f32,
}

impl DcBlock {
    pub fn new() -> Self {
        Self {
            x1: 0.0,
            y1: 0.0,
            r: 0.995,
        }
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + self.r * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}


// SOUND HELPERS
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


#[cfg(feature = "sounds")]
pub async fn play_ding() { play_sound(DING_SOUND).await; }
#[cfg(feature = "sounds")]
pub async fn play_done() { play_sound(DONE_SOUND).await; }
#[cfg(feature = "sounds")]
pub async fn play_fail() { play_sound(FAIL_SOUND).await; }
#[cfg(feature = "sounds")]
pub async fn beep_loop(active: &AtomicBool) {    
    while active.load(Ordering::Relaxed) {
        play_sound(BEEP_SOUND).await;
        Timer::after(Duration::from_secs(6)).await;
    }
}

// ───────────────────────────────────────────────────────────────────────
// MICROPHONE

pub struct Microphone {
    i2s_rx: I2sRx<'static, Async>,
    stereo_buffer: [u8; STEREO_SAMPLES_PER_READ * 2],
    accum_buffer: Vec<f32>,
    silent: bool,
    dc_block: DcBlock,
}

impl Microphone {
    pub fn new(i2s_rx: I2sRx<'static, Async>) -> Self {
        Self {
            i2s_rx,
            stereo_buffer: [0u8; STEREO_SAMPLES_PER_READ * 2],
            accum_buffer: Vec::with_capacity(OWW_MODEL_CHUNK_SIZE),
            silent: false,
            dc_block: DcBlock::new(),
        }
    }
   
    pub async fn read_chunk_into(&mut self, out: &mut [u8]) -> Result<bool, ()> {
        assert_eq!(out.len(), 4 + OWW_MODEL_CHUNK_SIZE * 4);
    
        // GATHER ENOUGH SAMPLES
        while self.accum_buffer.len() < OWW_MODEL_CHUNK_SIZE {
            let read_fut = self.i2s_rx.read_dma_async(&mut self.stereo_buffer);
            let timeout = embassy_time::Timer::after(embassy_time::Duration::from_secs(2));
            match embassy_futures::select::select(read_fut, timeout).await {
                embassy_futures::select::Either::First(Ok(())) => {}
                embassy_futures::select::Either::First(Err(e)) => {
                    error!("I2S read_dma_async failed: {:?}", Debug2Format(&e));
                    return Err(());
                }
                embassy_futures::select::Either::Second(_) => {
                    error!("I2S read_dma_async timed out (no data for 2s)");
                    return Err(());
                }
            }
    
            let mut stereo = [0i16; STEREO_SAMPLES_PER_READ];
            for (i, pair) in self.stereo_buffer.chunks_exact(2).enumerate() {
                stereo[i] = i16::from_le_bytes([pair[0], pair[1]]);
            }    

            // DEBUGGING
            #[cfg(debug_assertions)]
            {
                info!("[MIC i16]: {:?}", &stereo[..8.min(stereo.len())]);
            }
    
            for chunk in stereo.chunks(2) {
                let sum = chunk[0] as f32 + chunk[1] as f32;
                let mono_f32 = (sum / 2.0) / 32768.0;
                let filtered = self.dc_block.process(mono_f32);
                self.accum_buffer.push(filtered);
            }
        }
    

        // LENGTH PREFIX
        let len_bytes = (OWW_MODEL_CHUNK_SIZE as u32).to_le_bytes();
        out[..4].copy_from_slice(&len_bytes);

        let mut all_zero = true;
        for (i, &sample) in self.accum_buffer.iter().enumerate() {
            let offset = 4 + i * 4;
            out[offset..offset + 4].copy_from_slice(&sample.to_le_bytes());
            if sample != 0.0 {
                all_zero = false;
            }
        }

        // CLEAR THE ACCUMULATOR FOR THE NEXT CHUNK
        self.accum_buffer.clear();
    
        // SILENCE DETECTION LOGGING
        if all_zero {
            if !self.silent {
                info!("🎙️⚠️ Mic zero zero zero!");
                self.silent = true;
            }
        } else if self.silent {
            info!("🎙️✅ Mic OK!");
            self.silent = false;
        }
    
        Ok(all_zero)
    }
}    




// SEND RECORDED AUDIO TO THE SERVER USING THE PUSH-TO-TALK PROTOCOL (ROOM = "oneshot")
async fn _send_ptt_to_server(
    stack: &'static Stack<'static>,
    host: &str,
    port: u16,
    audio: &[f32],
    handler: &mut dyn CommandHandler,
) -> Result<(), ()> {
    let ip: core::net::Ipv4Addr = host.parse().map_err(|_| ())?;
    let remote_endpoint = (IpAddress::Ipv4(ip.octets().into()), port);

    let mut rx_buffer = [0u8; TCP_RX_BUF_SIZE];
    let mut tx_buffer = [0u8; TCP_TX_BUF_SIZE];
    let mut socket = TcpSocket::new(stack.clone(), &mut rx_buffer, &mut tx_buffer);
    // WAIT MAXIMUM OF 20 SECONDS - AFTER THAT COUT IT AS A FAILED COMMAND (INTERNALLY) - COMMAND MAY STILL EXECUTE ON BACKEND
    socket.set_timeout(Some(Duration::from_secs(20)));

    socket.connect(remote_endpoint).await.map_err(|e| {
        error!("PTT connect error: {:?}", e);
    })?;

    debug!("📡 ☑️ PTT connected to {}", host);

    // HANDSHAKE: SEND ROOM NAME "oneshot" FOR PUSH-TO-TALK HANDLING
    let room_bytes = b"oneshot";
    let room_len = room_bytes.len() as u32;
    let len_bytes = room_len.to_le_bytes();

    // HELPER CLOSURE TO WRITE ALL BYTES
    async fn write_all(socket: &mut TcpSocket<'_>, buf: &[u8]) -> Result<(), ()> {
        let mut written = 0;
        while written < buf.len() {
            let n = socket.write(&buf[written..]).await.map_err(|e| {
                error!("write error: {:?}", e);
            })?;
            written += n;
        }
        Ok(())
    }

    write_all(&mut socket, &len_bytes).await?;
    write_all(&mut socket, room_bytes).await?;
    socket.flush().await.map_err(|e| {
        error!("PTT flush error: {:?}", e);
    })?;

    // NOTIFY HANDLER (THINKING)
    handler.on_thinking().await;

    // PTT PROTOCOL
    // PTT_START
    write_all(&mut socket, &[0x10]).await?;

    // PTT_DATA
    let chunk_size = 512; // f32 SAMPLES PER CHUNK
    for chunk in audio.chunks(chunk_size) {
        let num = chunk.len() as u32;
        let mut header = [0u8; 5];
        header[0] = 0x11;
        header[1..5].copy_from_slice(&num.to_le_bytes());
        write_all(&mut socket, &header).await?;

        let mut byte_vec: Vec<u8> = Vec::with_capacity(chunk.len() * 4);
        for &sample in chunk {
            byte_vec.extend_from_slice(&sample.to_le_bytes());
        }
        write_all(&mut socket, &byte_vec).await?;
        
    }

    // PTT_END
    write_all(&mut socket, &[0x12]).await?;

    // READ SERVER RESPONSE (one byte)
    let mut resp = [0u8; 1];
    match socket.read(&mut resp).await {
        Ok(1) => {
            match resp[0] {
                0x03 => {
                    info!("✅ PTT command executed successfully");
                    handler.on_executed(None).await;
                }
                0x04 => {
                    error!("💩 PTT command failed (server reported)");
                    handler.on_failed(None).await;
                }
                _ => {
                    error!("Unexpected PTT response: 0x{:02x}", resp[0]);
                    handler.on_failed(None).await;
                }
            }
        }
        _ => {
            error!("No response from PTT server");
            handler.on_failed(None).await;
        }
    }

    let _ = socket.close();
    Ok(())
}


// MICROPHONE TASK
#[embassy_executor::task]
pub async fn audio_capture_task(
    i2s_rx: I2sRx<'static, Async>,
    stack: &'static Stack<'static>,
    host: &'static str,
    port: u16,
    room: &'static str,
    mut handler: alloc::boxed::Box<dyn CommandHandler>,
) {
    let ip: core::net::Ipv4Addr = match host.parse() {
        Ok(ip) => ip,
        Err(_) => {
            error!("Invalid host IP address: {}", host);
            return;
        }
    };
    let remote_addr = SocketAddr::V4(core::net::SocketAddrV4::new(ip, port));
    let remote_endpoint = (IpAddress::Ipv4(ip.octets().into()), port);

    let mut mic = Microphone::new(i2s_rx);
    let room_bytes = room.as_bytes();
    let room_len = room_bytes.len() as u32;

    let mut chunk_buf = [0u8; 4 + OWW_MODEL_CHUNK_SIZE * 4];

    loop {
        // WAIT FOR ENABLED COMMAND (idle, no audio streaming)
        info!("🎙️  💤");
        let cmd = VOICE_CMD.receive().await; // BLOCKS UNTIL `Enabled` 
        debug!("Received command: {:?}", cmd);
        
        match cmd {
            // COMMAND: `Enabled` - ENABLES WAKE WORD DETECTION/STREAMS MICROPHONE AUDIO
            VoiceCommand::Enabled => {
                // NOW ENABLED
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
        
                let mut command_start: Option<Instant> = None;
                let mut disabled = false;        
        
                'stream: loop {
                    // CHECK FOR DISABLED CMD WITHOUT BLOCKING
                    if let Ok(VoiceCommand::Disabled) = VOICE_CMD.try_receive() {
                        disabled = true;
                        info!("Disabled – exiting wake‑word mode");
                        Timer::after(Duration::from_millis(10)).await;
                        break 'stream;
                    }
                    // FILL THE PACKET BUFFER – NO ALLOCATION!
                    let _silent = match mic.read_chunk_into(&mut chunk_buf).await {
                        Ok(v) => v,
                        Err(_) => {
                            error!("I2S read ERROR");
                            Timer::after(Duration::from_millis(10)).await;
                            continue;
                        }
                    };
        
                    // SEND THE PACKET DIRECTLY
                    let mut written = 0;
                    while written < chunk_buf.len() {
                        match socket.write(&chunk_buf[written..]).await {
                            Ok(n) => written += n,
                            Err(e) => {
                                error!("failed to send audio chunk: {:?}", e);
                                break 'stream;
                            }
                        }
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
                                    } else { info!("✅ Executed command!"); }
                                    handler.on_executed(elapsed).await;
                                    command_start = None;
                                }
                                0x04 => {
                                    let elapsed = command_start.map(|s| s.elapsed().as_millis());
                                    if let Some(ms) = elapsed {
                                        info!("💩 FAILED execution ({} ms)", ms);
                                    } else { info!("💩 FAILED execution!"); }
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
        
                let _ = socket.close();
                if disabled {
                    info!("Disabled – returning to idle");
                } else {
                    info!("❌ reconnecting to yo in 15 seconds...");
                    Timer::after(Duration::from_secs(15)).await;
                }
            }
            
            // COMMAND: `Pushed` - USED TO SIGNAL START OF PUSH-TO-TALK
            VoiceCommand::Pushed => {
                let mut rx_buffer = [0u8; TCP_RX_BUF_SIZE];
                let mut tx_buffer = [0u8; TCP_TX_BUF_SIZE];
                let mut socket = TcpSocket::new(*stack, &mut rx_buffer, &mut tx_buffer);
                socket.set_timeout(Some(Duration::from_secs(20)));
            
                let remote_endpoint = (IpAddress::Ipv4(ip.octets().into()), port);
                if let Err(e) = socket.connect(remote_endpoint).await {
                    error!("yo ptt connect error: {:?}", e);
                    continue; // BACK TO IDLE
                }
            
                debug!("PTT connected to {}", host);
            
                // HANDSHAKE: USE `oneshot` AS ROOM NAME! (TODO: remove the oneshot room name requirement from backend)
                let room_bytes = b"oneshot";
                let room_len = room_bytes.len() as u32;
            
                async fn write_all(socket: &mut TcpSocket<'_>, buf: &[u8]) -> Result<(), ()> {
                    let mut written = 0;
                    while written < buf.len() {
                        let n = socket.write(&buf[written..]).await.map_err(|e| {
                            error!("PTT write error: {:?}", e);
                        })?;
                        written += n;
                    }
                    Ok(())
                }
            
                if write_all(&mut socket, &room_len.to_le_bytes()).await.is_err()
                    || write_all(&mut socket, room_bytes).await.is_err()
                {
                    error!("PTT handshake failed");
                    let _ = socket.close();
                    continue;
                }
            
                // SEND `0x10` PTT_START - TO BACKEND TO SIGNAL THIS IS PUSH-TO-TALK
                if write_all(&mut socket, &[0x10]).await.is_err() {
                    error!("Failed to send PTT_START");
                    let _ = socket.close();
                    continue;
                }
                          
                let record_start = Instant::now();
                let mut total_samples = 0usize;
                
                // RECORD AND STREAMING LOOP
                'record: loop {
                    // READ ONE CHUNK FROM THE MIC
                    let _silent = match mic.read_chunk_into(&mut chunk_buf).await {
                        Ok(v) => v,
                        Err(_) => {
                            error!("I2S read error – aborting PTT");
                            break 'record;
                        }
                    };
            
                    // chunk_buf layout: [4 bytes length][OWW_MODEL_CHUNK_SIZE * 4 bytes f32]
                    let f32_bytes = &chunk_buf[4..4 + OWW_MODEL_CHUNK_SIZE * 4];
            
                    // BUILD `0x11` -  PTT_DATA HEADER
                    let num = OWW_MODEL_CHUNK_SIZE as u32;
                    let mut header = [0u8; 5];
                    header[0] = 0x11;
                    header[1..5].copy_from_slice(&num.to_le_bytes());
            
                    if write_all(&mut socket, &header).await.is_err()
                        || write_all(&mut socket, f32_bytes).await.is_err()
                    {
                        error!("Failed to send PTT_DATA chunk");
                        break 'record;
                    }
            
                    total_samples += OWW_MODEL_CHUNK_SIZE;
            
                    // STOP OR NOT? CHECK `Released` COMMAND
                    if let Ok(VoiceCommand::Released) = VOICE_CMD.try_receive() {
                        break 'record;
                    }
                }
            
                let record_end = Instant::now();
                let duration_ms = (record_end - record_start).as_millis();
                debug!("Mic streamed {} samples - {} ms duration", total_samples, duration_ms);
            
                // SEND PTT_END `0x12` TO BACKEND
                if write_all(&mut socket, &[0x12]).await.is_err() {
                    error!("Failed to send PTT_END");
                }
                info!("🧠 THINKING...");
            
                // READ SERVER RESPONSE (1 byte)
                let mut resp = [0u8; 1];
                match socket.read(&mut resp).await {
                    Ok(1) => {
                        match resp[0] {
                            // `0x03` == SUCCESSFUL EXECUTION !
                            0x03 => {
                                info!("✅ Command executed successfully");
                                handler.on_executed(None).await;
                            }
                            // `0x04` == FAILED EXECUTION!
                            0x04 => {
                                error!("💩 Command execution failed!");
                                handler.on_failed(None).await;
                            }
                            _ => {
                                error!("Unexpected PTT response: 0x{:02x}", resp[0]);
                                handler.on_failed(None).await;
                            }
                        }
                    }
                    Ok(_) => error!("PTT server closed connection early"),
                    Err(e) => error!("PTT response read error: {:?}", e),
                }
                let _ = socket.close();
            }
            
            
            // COMMAND: `Intercom` – STREAM MICROPHONE AUDIO TO THE BACKEND SPEAKER (ENABLE STREAMING TASK TO MAKE IT BIDIRECTIONAL) 
            VoiceCommand::Intercom => {
                let mut rx_buffer = [0u8; TCP_RX_BUF_SIZE];
                let mut tx_buffer = [0u8; TCP_TX_BUF_SIZE];
                let mut socket = TcpSocket::new(stack.clone(), &mut rx_buffer, &mut tx_buffer);
                socket.set_timeout(Some(Duration::from_secs(10)));
            
                if let Err(e) = socket.connect(remote_endpoint).await {
                    error!("❌ Intercom connect error: {:?}, retrying in 5s", e);
                    Timer::after(Duration::from_secs(5)).await;
                    continue;
                }
                info!("📡 Intercom streaming to {}", remote_addr);
            
                // HANDSHAKE WITH FORCED ROOM NAME: "intercom"
                let intercom_room = b"intercom";
                let intercom_len = intercom_room.len() as u32;
            
                let len_bytes = intercom_len.to_le_bytes();
                let mut written = 0;
                while written < len_bytes.len() {
                    match socket.write(&len_bytes[written..]).await {
                        Ok(n) => written += n,
                        Err(e) => {
                            error!("intercom handshake length fail: {:?}", e);
                            break;
                        }
                    }
                }
            
                if written == len_bytes.len() {
                    let mut written = 0;
                    while written < intercom_room.len() {
                        match socket.write(&intercom_room[written..]).await {
                            Ok(n) => written += n,
                            Err(e) => {
                                error!("failed to send intercom room: {:?}", e);
                                break;
                            }
                        }
                    }
                }
                if let Err(e) = socket.flush().await {
                    error!("failed to flush intercom handshake: {:?}", e);
                }
            
                // STREAM MIC CHUNKS
                'stream: loop {
                    // CHECK FOR `Disabled` COMMAND WITHOUT BLOCKING
                    if let Ok(VoiceCommand::Disabled) = VOICE_CMD.try_receive() {
                        info!("Intercom disabled – returning to idle");
                        break 'stream;
                    }
            
                    // READ ONE CHUNK FROM THE MIC
                    let _silent = match mic.read_chunk_into(&mut chunk_buf).await {
                        Ok(v) => v,
                        Err(_) => {
                            error!("I2S read error in intercom");
                            Timer::after(Duration::from_millis(10)).await;
                            continue;
                        }
                    };
            
                    // SEND CHUNK
                    let mut written = 0;
                    while written < chunk_buf.len() {
                        match socket.write(&chunk_buf[written..]).await {
                            Ok(n) => written += n,
                            Err(e) => {
                                error!("Intercom send error: {:?}", e);
                                break 'stream;
                            }
                        }
                    }
                }
            
                let _ = socket.close();
                info!("Intercom ended – idle");
            }
                        
            // COMMAND: `Released` - USED TO SIGNAL END OF PUSH-TO-TALK
            VoiceCommand::Released => {
                debug!("Released while idle, ignoring");
            }
            
            // COMMAND: `Disabled` - USED TO STOP WAKE WORD DETECTION
            VoiceCommand::Disabled => {
                debug!("Disabled while idle, ignoring");
            }            
        }
    }
}


// ───────────────────────────────────────────────────────────────────────
// SPEAKER
static PIPE: Pipe<CriticalSectionRawMutex, RING_BUFFER_SIZE> = Pipe::new();

// SPEAKER TASK
#[embassy_executor::task]
pub async fn speaker_task(
    mut transfer: I2sWriteDmaTransferAsync<'static, &'static mut [u8; SPEAKER_DMA_BUFFER_SIZE]>,
) -> ! {
    let mut pipe_buf = [0u8; 256];
    let silence = [0u8; 256];

    loop {
        // IDLE - WAIT FOR START SIGNAL
        info!("🔊 💤");
        //SPEAKER_CMD.send(SpeakerCommand::Start).await;
        let cmd = SPEAKER_CMD.receive().await; // BLOCKS UNTIL `Start` 
        debug!("Received command: {:?}", cmd);
        
        match cmd {    
            SpeakerCommand::Start => {
                let mut _stopped = false;  
                info!("🔊 ☑️");         
                
                'stream: loop {
                    // CHECK FOR STOP SIGNAL WITHOUT BLOCKING
                    if let Ok(SpeakerCommand::Stop) = SPEAKER_CMD.try_receive() {
                        _stopped = true;
                        info!("Pausing speaker task");
                        break 'stream;
                    }
            
                    let free = match transfer.available().await {
                        Ok(free) => free,
                        Err(e) => {
                            error!("DMA available error: {:?}", e);
                            //panic!("I2S TX IS DEAD!");
                            break 'stream;
                        }
                    };
                    if free == 0 {
                        Timer::after(Duration::from_micros(1)).await;
                        continue;
                    }

                    let to_read = free.min(pipe_buf.len());
                    let read_future = PIPE.read(&mut pipe_buf[..to_read]);
                    let timeout = Timer::after(Duration::from_millis(1));

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
            SpeakerCommand::Stop => { debug!("Stopping speaker task"); }
        }
    }
}     


// STREAMING TASK
#[embassy_executor::task]
pub async fn stream_speaker(
    stack: &'static Stack<'static>,
    listen_port: u16,
) {
    debug!("Speaker streaming task started");

    loop {
        // IDLE - WAIT FOR START SIGNAL
        let cmd = STREAM_CMD.receive().await; // BLOCKS UNTIL `StreamCommand::Start`
        debug!("Received command: {:?}", cmd);

        match cmd {
            StreamCommand::Start => {
                // NOW STREAMING TO THE ESP32 IS ALLOWED
                stack.wait_link_up().await;
                stack.wait_config_up().await;
                MEDIA_IS_PLAYING.store(false, Ordering::Release);
                info!("📡 🔊 💤");

                loop {
                    // WAITING FOR CLIENT
                    let mut rx_buffer = [0u8; PLAYBACK_TCP_RX_BUF_SIZE];
                    let mut tx_buffer = [0u8; PLAYBACK_TCP_TX_BUF_SIZE];
                    let mut socket = TcpSocket::new(stack.clone(), &mut rx_buffer, &mut tx_buffer);

                    let accept_fut = socket.accept(listen_port);
                    let stop_fut = STREAM_CMD.receive();
                    
                    // ACCEPT WITH STOP CHECKING
                    match select(accept_fut, stop_fut).await {
                        Either::First(Ok(())) => {
                            // CLIENT CONNECTED!
                            info!("📡 ☑️ 🔊");
                            MEDIA_IS_PLAYING.store(true, Ordering::Release);
                            socket.set_timeout(Some(Duration::from_secs(30)));
                        }
                        Either::First(Err(e)) => {
                            error!("accept error: {:?}", e);
                            Timer::after(Duration::from_secs(1)).await;
                            continue;
                        }
                        Either::Second(StreamCommand::Stop) => {
                            // `Stop` BEFORE CONNECTION
                            info!("📡 🚧 🔇");
                            break; // EXIT INNER LOOP AND RETURN TO IDLE
                        }
                        Either::Second(_) => {
                            // ??? COMMAND IGNORE IT!
                            continue;
                        }
                    }

                    // READ LOOP WITH INACTIVITY TIMER & STOP CHECKING
                    let mut buf = [0u8; 1024];
                    let inactivity_limit = Duration::from_secs(5);

                    'read: loop {
                        let read_fut = socket.read(&mut buf);
                        let stop_fut = STREAM_CMD.receive();
                        let timeout_fut = Timer::after(inactivity_limit);

                        // WAIT FOR DATA `Stop` COMMAND OR TIMEOUT
                        match select(read_fut, select(stop_fut, timeout_fut)).await {
                            // DATA RECEIVED
                            Either::First(Ok(0)) => {
                                // CLEAN CLIENT EXIT
                                break 'read;
                            }
                            Either::First(Ok(n)) => {
                                // ROUTE THE DATA TO SPEAKER
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
                            Either::First(Err(e)) => {
                                error!("read error: {:?}", e);
                                break 'read;
                            }
                            // `Stop` OR TIMEOUT
                            Either::Second(Either::First(StreamCommand::Stop)) => {
                                info!("📡 🚧 🔇");
                                MEDIA_IS_PLAYING.store(false, Ordering::Release);
                                break 'read;
                            }
                            Either::Second(Either::Second(_)) => {
                                // INACTIVITY
                                MEDIA_IS_PLAYING.store(false, Ordering::Release);
                                debug!("idle timeout disconnecting");
                                break 'read;
                            }
                            _ => {}
                        }
                    }

                    MEDIA_IS_PLAYING.store(false, Ordering::Release);
                    let _ = socket.close();
                    info!("🔇 💤");
                }
            }
            StreamCommand::Stop => {
                debug!("already idle, ignoring");
            }
        }
    }
}
