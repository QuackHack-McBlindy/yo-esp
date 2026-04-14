#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::net::SocketAddr;

use embassy_futures::select::{select, Either};
use embassy_net::{tcp::TcpSocket, IpAddress, Stack};
use embassy_time::{Duration, Timer};


#[cfg(feature = "defmt")]
use defmt::{error, info};

#[cfg(not(feature = "defmt"))]
macro_rules! info {
    ($($arg:tt)*) => {{}};
}
#[cfg(not(feature = "defmt"))]
macro_rules! error {
    ($($arg:tt)*) => {{}};
}


#[derive(Debug, Clone)]
pub struct AudioStreamConfig {
    pub room: &'static str,
    pub chunk_size: usize,
    pub rx_buf_size: usize,
    pub tx_buf_size: usize,
    pub timeout: Duration,
    pub reconnect_delay: Duration,
}

impl Default for AudioStreamConfig {
    fn default() -> Self {
        Self {
            room: "esp",
            chunk_size: 1280,
            rx_buf_size: 1024,
            tx_buf_size: 4096,
            timeout: Duration::from_secs(10),
            reconnect_delay: Duration::from_secs(15),
        }
    }
}

pub type AudioChunk = Vec<f32>;

pub trait AudioSource {
    type Error;
    async fn read_chunk(&mut self) -> Result<(AudioChunk, bool), Self::Error>;
}


pub trait CommandHandler {

    fn on_wake_word_detected(&mut self);
    fn on_command_executed(&mut self);
    fn on_command_failed(&mut self);
}


/// The main audio streaming loop.
///
/// This function is generic over the `AudioSource` and `CommandHandler` types.
/// You should wrap it in an `#[embassy_executor::task]` with concrete types
/// before spawning.
///
/// # Example
/// ```ignore
/// #[embassy_executor::task]
/// async fn audio_task(
///     mic: Microphone,
///     handler: MyHandler,
///     stack: &'static Stack<'static>,
///     addr: SocketAddr,
///     config: AudioStreamConfig,
/// ) {
///     yo_esp::run_audio_stream(mic, handler, stack, addr, config).await;
/// }
///
/// spawner.spawn(audio_task(mic, handler, &stack, remote, config)).unwrap();
/// ```
pub async fn run_audio_stream<S, H>(
    mut source: S,
    mut handler: H,
    stack: &'static Stack<'static>,
    remote_addr: SocketAddr,
    config: AudioStreamConfig,
) where
    S: AudioSource,
    H: CommandHandler,
{
    let remote_endpoint = match remote_addr {
        SocketAddr::V4(v4) => (IpAddress::Ipv4(v4.ip().octets().into()), v4.port()),
        SocketAddr::V6(_) => {
            error!("IPv6 not supported");
            return;
        }
    };

    stack.wait_link_up().await;
    stack.wait_config_up().await;

    let room_bytes = config.room.as_bytes();
    let room_len = room_bytes.len() as u32;

    loop {
        let mut rx_buffer = alloc::vec![0u8; config.rx_buf_size];
        let mut tx_buffer = alloc::vec![0u8; config.tx_buf_size];
        let mut socket = TcpSocket::new(stack.clone(), &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(config.timeout));

        if let Err(e) = socket.connect(remote_endpoint).await {
            error!(
                "❌ connect error: {:?}, retrying in {}s",
                e,
                config.reconnect_delay.as_secs()
            );
            Timer::after(config.reconnect_delay).await;
            continue;
        }
        info!("📡 ☑️ 🎙️ to {}", remote_addr);

        // Handshake
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
            Timer::after(config.reconnect_delay).await;
            continue;
        }

        // Streaming loop
        'stream: loop {
            let (chunk, _silent) = match source.read_chunk().await {
                Ok(pair) => pair,
                Err(_) => {
                    error!("audio source read error");
                    Timer::after(Duration::from_millis(10)).await;
                    continue;
                }
            };

            let mut chunk_buffer = alloc::vec![0u8; 4 + config.chunk_size * 4];
            chunk_buffer[0..4].copy_from_slice(&(config.chunk_size as u32).to_le_bytes());
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
                Either::First(Ok(1)) => match byte_buf[0] {
                    0x01 => handler.on_wake_word_detected(),
                    0x03 => handler.on_command_executed(),
                    0x04 => handler.on_command_failed(),
                    _ => info!("Unexpected byte: 0x{:02x}", byte_buf[0]),
                },
                Either::First(Ok(_)) => {}
                Either::First(Err(e)) => {
                    error!("socket read error: {:?}", e);
                    break 'stream;
                }
                Either::Second(_) => {}
            }
        }

        info!("❌ reconnecting...");
        let _ = socket.close();
        Timer::after(config.reconnect_delay).await;
    }
}


#[cfg(feature = "i2s")]
pub mod microphone;
