use alloc::vec::Vec;
use esp_hal::i2s::master::I2sRx;
use esp_hal::Async;

use crate::{AudioChunk, AudioSource};

#[cfg(feature = "defmt")]
use defmt::info;
#[cfg(not(feature = "defmt"))]
macro_rules! info {
    ($($arg:tt)*) => {{}};
}

const STEREO_SAMPLES_PER_READ: usize = 256;
const MONO_SAMPLES_PER_READ: usize = STEREO_SAMPLES_PER_READ / 2;
const DEFAULT_CHUNK_SIZE: usize = 1280;

/// I2S microphone that accumulates mono `f32` samples
///
/// the microphone is assumed to provide stereo 16‑bit PCM data; it averages
/// the two channels to produce a mono signal and normalises to `f32` in `[-1.0, 1.0]`.
pub struct Microphone {
    i2s_rx: I2sRx<'static, Async>,
    stereo_buffer: [u8; STEREO_SAMPLES_PER_READ * 2],
    mono_i16: [i16; MONO_SAMPLES_PER_READ],
    mono_f32: [f32; MONO_SAMPLES_PER_READ],
    accum_buffer: Vec<f32>,
    silent: bool,
    chunk_size: usize,
}

impl Microphone {
    /// create a new microphone instance with the default chunk size (1280).
    pub fn new(i2s_rx: I2sRx<'static, Async>) -> Self {
        Self::with_chunk_size(i2s_rx, DEFAULT_CHUNK_SIZE)
    }

    /// create a new microphone with a custom chunk size.
    ///
    /// the chunk size must be a multiple of `MONO_SAMPLES_PER_READ` (128)
    /// for correct operation, though the implementation will work with any size.
    pub fn with_chunk_size(i2s_rx: I2sRx<'static, Async>, chunk_size: usize) -> Self {
        Self {
            i2s_rx,
            stereo_buffer: [0u8; STEREO_SAMPLES_PER_READ * 2],
            mono_i16: [0i16; MONO_SAMPLES_PER_READ],
            mono_f32: [0f32; MONO_SAMPLES_PER_READ],
            accum_buffer: Vec::with_capacity(chunk_size),
            silent: false,
            chunk_size,
        }
    }
}

impl AudioSource for Microphone {
    type Error = ();

    async fn read_chunk(&mut self) -> Result<(AudioChunk, bool), Self::Error> {
        while self.accum_buffer.len() < self.chunk_size {
            // read one stereo block from I2S
            if self
                .i2s_rx
                .read_dma_async(&mut self.stereo_buffer)
                .await
                .is_err()
            {
                return Err(());
            }

            // interpret raw bytes as i16 samples (little‑endian).
            let stereo = unsafe {
                core::slice::from_raw_parts(
                    self.stereo_buffer.as_ptr() as *const i16,
                    STEREO_SAMPLES_PER_READ,
                )
            };

            // average left/right pairs > mono i16
            for (i, chunk) in stereo.chunks(2).enumerate() {
                self.mono_i16[i] = ((chunk[0] as i32 + chunk[1] as i32) / 2) as i16;
            }

            // convert to f32 in range [-1.0, 1.0].
            for (i, &sample) in self.mono_i16.iter().enumerate() {
                self.mono_f32[i] = sample as f32 / 32768.0;
            }

            self.accum_buffer
                .extend_from_slice(&self.mono_f32[..MONO_SAMPLES_PER_READ]);
        }

        // Drain exactly `chunk_size` samples.
        let chunk: Vec<f32> = self.accum_buffer.drain(..self.chunk_size).collect();

        // Silence Detection
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
