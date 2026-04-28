# **yo-esp**

[![Sponsors](https://img.shields.io/github/sponsors/QuackHack-McBlindy?logo=githubsponsors&label=Sponsor&style=flat&labelColor=ff1493&logoColor=fff&color=rgba(234,74,170,0.5) "")](https://github.com/sponsors/QuackHack-McBlindy) [![Buy Me a Coffee](https://img.shields.io/badge/Buy%20Me%20a%20Coffee-Sponsor?style=flat&logo=buymeacoffee&logoColor=fff&labelColor=ff1493&color=ff1493)](https://buymeacoffee.com/quackhackmcblindy)


`yo-esp` is a **bare‑metal, `no_std` audio streaming client** for the [**yo** voice assistant](https://github.com/QuackHack-McBlindy/yo).  
It runs on the **ESP32‑S3** and provides everything needed to capture microphone audio, stream it to a backend server, receive and play back TTS/audio, and react to server‑side wake‑word detection, speech‑to‑text, intent recognition and command execution.  
   

> **What it does**  
> * Streams 16 kHz mono audio from an I²S microphone (using the ES7210 ADC driver) to a remote wake‑word / STT / intent server over TCP.  
> * Receives audio from the server (TTS, server-side files, HTTP/HTTPS, playlists) and plays it through the I²S speaker (ES8311 DAC).  
> * Dispatches callbacks for wake‑word detected, thinking, command executed, and command failed events.  
> * Includes built‑in “ding”, “done” and “fail” notification sounds.  
> * Completely `no_std`, runs on bare metal with `embassy‑net` and `esp‑hal`.  


## **Installation**

  
Add `yo-esp` as a dependency in `Cargo.toml`.  

```toml
[dependencies]
yo-esp = "0.1.2"
```

You will also need a compatible network stack (embassy-net), an I²S driver (esp-hal), and the codec drivers (es7210, es8311).  
Example `Cargo.toml`:  

```toml
[dependencies]
yo-esp = "0.1.2"
embassy-net = { version = "0.5", features = ["tcp", "udp", "dhcpv4", "dns"] }
esp-hal = { version = "0.22", features = ["async", "esp32s3"] }
es7210 = "0.1.0"
es8311 = "0.1.0"
embedded-hal = "1.0"
defmt = "0.3"
```

<br>

## **Example usage**


A minimal `main.rs` that sets up Wi‑Fi, the network stack, I²S, the codecs, and spawns the `yo-esp` tasks:  

```rust
#![no_std]
#![no_main]

use yo_esp::{audio_capture_task, speaker_task, stream_speaker, CommandHandler, play_ding, play_done, play_fail};

// Implement your own callbacks
struct VoiceHandler;

impl CommandHandler for VoiceHandler {
    fn on_detected(&mut self) -> core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = ()> + '_>> {
        Box::pin(async {
            // Wake word heard – play a ding and turn on the display
            play_ding().await;
            // crate::components::display::brightness_set("70");
        })
    }

    fn on_thinking(&mut self) -> core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = ()> + '_>> {
        Box::pin(async {
            // Server is processing speech
        })
    }

    fn on_executed(&mut self, ms: Option<u64>) -> core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = ()> + '_>> {
        Box::pin(async move {
            // Command executed successfully
            play_done().await;
        })
    }

    fn on_failed(&mut self, ms: Option<u64>) -> core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = ()> + '_>> {
        Box::pin(async move {
            // Command failed
            play_fail().await;
        })
    }
}

#[esp_rtos::main]
async fn main(spawner: embassy_executor::Spawner) -> ! {
    // ... (set up Wi‑Fi, I²C, I²S, codecs; see the full example in the repository) ...

    let handler: alloc::boxed::Box<dyn CommandHandler> = alloc::boxed::Box::new(VoiceHandler);

    // Start the speaker DMA pump
    spawner.spawn(speaker_task(i2s_tx_transfer)).ok();
    // Route TCP 12345 to the speaker
    spawner.spawn(stream_speaker(stack, 12345)).ok();
    // Route TCP 12345 from microphone to server
    // A bidirectional connection is established. 
    spawner.spawn(audio_capture_task(i2s_rx, stack, remote_addr, "esp", handler)).ok();

    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(60)).await;
    }
}
```

> [!NOTE]
> A complete, runnable example can be found in the `ESP32‑S3‑BOX‑3-rs` [repository](https://github.com/QuackHack-McBlindy/ESP32-S3-BOX-3-rs).  

  

## **API overview**

### **`CommandHandler` trait**

| Method | Server byte | Meaning |
|--------|-------------|---------|
| `on_detected()` | `0x01` | Wake word detected |
| `on_thinking()` | `0x02` | Speech‑to‑text / intent processing has begun |
| `on_executed(elapsed_ms)` | `0x03` | Command was executed successfully |
| `on_failed(elapsed_ms)` | `0x04` | Command execution failed |

### **Tasks**

| Task | Description |
|------|-------------|
| `audio_capture_task(i2s_rx, stack, remote_addr, room, handler)` | Streams microphone audio to the server and dispatches `CommandHandler` callbacks. |
| `speaker_task(transfer)` | Pumps audio data from an internal ring buffer (`PIPE`) to the I²S DAC. |
| `stream_speaker(stack, listen_port)` | Accepts a TCP connection on `listen_port` and writes incoming audio into the ring buffer. |

### **Sound helpers**

| Function | Plays |
|----------|-------|
| `play_ding()` | The “ding” notification sound |
| `play_done()` | The “done” success sound |
| `play_fail()` | The “fail” error sound |

You can also push arbitrary audio using `play(data: &[u8])`.  

> [!NOTE]  
> **I included a helper script for streaming various audio types to the ESP32‑S3**  
> **Supports streaming desktop microphone to ESP32-S3 for intercom mode.**  
> **You will find helper at: `examples/esp-play.sh`**   


###  **Hardware / platform requirements**

- **ESP32‑S3 (the library uses esp‑hal I²S and DMA).**  

- **ES7210 quad ADC for microphone input (also works with other I²S microphones; adapt the codec driver).**  

- **ES8311 codec for speaker output.**  

- **Wi‑Fi connectivity through embassy‑net + esp‑radio.**  

- **`embassy‑executor` for async tasks.**  

  

## **Architecture**


```
                  ┌─────────────────────────────┐
                  │     yo-esp (ESP32‑S3)        │
 Microphone ──────┤ I²S RX ──► audio_capture_task│── TCP ──► yo server (STT, TTS, intent)
                  │                              │
       Speaker ◄──┤ I²S TX ◄── speaker_task      │◄─ TCP ─── (Any audio)
                  │          stream_speaker      │
                  └─────────────────────────────┘
```

> * `audio_capture_task` reads I²S, converts to mono `f32`, buffers into chunks of `1280` samples (matching the wake‑word model), and sends them to the server.  
> * The server replies with a single byte per chunk to signal wake‑word detection / status events.  
> * `stream_speaker` receives raw PCM data over TCP and pushes it into a lock‑free pipe.  
> * `speaker_task` dequeues from that pipe and writes it to the I²S TX DMA.  


<br><br>

## **☕**

[![Sponsors](https://img.shields.io/github/sponsors/QuackHack-McBlindy?logo=githubsponsors&label=Sponsor&style=flat&labelColor=ff1493&logoColor=fff&color=rgba(234,74,170,0.5) "")](https://github.com/sponsors/QuackHack-McBlindy) [![Buy Me a Coffee](https://img.shields.io/badge/Buy%20Me%20a%20Coffee-Sponsor?style=flat&logo=buymeacoffee&logoColor=fff&labelColor=ff1493&color=ff1493)](https://buymeacoffee.com/quackhackmcblindy)
> 🦆🧑‍🦯 says ⮞ Hi! I'm QuackHack-McBlindy!  
> Like my work?  
> Buy me a coffee, or become a sponsor.  
> Thanks for supporting open source/hungry developers ♥️🦆!   

♥️₿ *Wallet:* `pungkula.x`  
<a href="https://www.buymeacoffee.com/quackhackmcblindy" target="_blank"><img src="https://cdn.buymeacoffee.com/buttons/v2/default-yellow.png" alt="Buy Me A Coffee" style="height: 60px !important;width: 217px !important;" ></a>

<br>

## **License**

This project is licensed under the terms of the MIT license.  
See the `LICENSE` file in the repository for full details.   
