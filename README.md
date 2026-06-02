# **yo-esp**

[![Sponsors](https://img.shields.io/github/sponsors/QuackHack-McBlindy?logo=githubsponsors&label=Sponsor&style=flat&labelColor=ff1493&logoColor=fff&color=rgba(234,74,170,0.5) "")](https://github.com/sponsors/QuackHack-McBlindy) [![Buy Me a Coffee](https://img.shields.io/badge/Buy%20Me%20a%20Coffee-Sponsor?style=flat&logo=buymeacoffee&logoColor=fff&labelColor=ff1493&color=ff1493)](https://buymeacoffee.com/quackhackmcblindy)


`yo-esp` is a **bare‑metal, `no_std` audio streaming client** for the [**yo** voice assistant](https://github.com/QuackHack-McBlindy/yo).  
It runs on the **ESP32‑S3** and provides everything needed to capture microphone audio, stream it to a backend server, receive and play back TTS/audio, and react to server‑side wake‑word detection, speech‑to‑text, intent recognition and command execution.  
   

> **What it does**  
> * Streams 16 kHz mono audio from an I²S microphone to a remote wake‑word / STT / intent server over TCP.  
> * Supports **push‑to‑talk** mode – records while a button is held and sends the full utterance in one go.  
> * Receives audio from the server (TTS, music, etc.) and plays it through the I²S speaker.  
> * Dispatches callbacks for wake‑word detected, thinking, command executed, and command failed events.  
> * Built‑in “ding”, “done” and “fail” notification sounds (enable with feature `sounds`).  
> * Completely `no_std`, runs on bare metal with `embassy‑net`, `esp‑hal` and `embassy‑executor`.  
> * All internal tasks are controllable from anywhere via global `embassy‑sync` channels.  


## **Installation**

  
Add `yo-esp` as a dependency in `Cargo.toml`.  

```toml
[dependencies]
yo-esp = "0.1.5"
```

You will also need a compatible network stack (embassy-net), an I²S driver (esp-hal), and the codec drivers (es7210, es8311).  
Example `Cargo.toml`:  

```toml
[dependencies]
yo-esp = "0.1.5"
embassy-executor = { version = "0.10.0", features = ["defmt"] }
esp-radio = { version = "0.18.0", features = [ "ble", "coex", "defmt", "esp-alloc", "esp32s3", "unstable", "wifi", ] }
embassy-net = { version = "0.9.0", features = [ "defmt", "dns", "dhcpv4", "medium-ethernet", "tcp", "udp", ] }
esp-hal  = { version = "1.1.1", features = ["defmt", "esp32s3", "unstable"] }
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
        // ... set up Wi‑Fi, I²S RX/TX, codecs ...

        let handler: alloc::boxed::Box<dyn CommandHandler> = alloc::boxed::Box::new(VoiceHandler);

        spawner.spawn(speaker_task(i2s_tx_transfer)).ok();
        spawner.spawn(stream_speaker(stack, 12345)).ok();
        spawner.spawn(audio_capture_task(i2s_rx, stack, "192.168.1.100", 54321, "esp", handler)).ok();

        // Optional: a push-to-talk button task
        #[embassy_executor::task]
        async fn push_to_talk(mut button: esp_hal::gpio::Input<'static>) {
            loop {
                button.wait_for_low().await;
                let _ = VOICE_CMD.send(VoiceCommand::Pushed).await;
                button.wait_for_high().await;
                let _ = VOICE_CMD.send(VoiceCommand::Released).await;
            }
        }

        loop {
            embassy_time::Timer::after(embassy_time::Duration::from_secs(60)).await;
        }
    }

> [!NOTE]
> A complete, runnable example can be found in the `ESP32‑S3‑WATCH-rs` [repository](https://github.com/QuackHack-McBlindy/ESP32-S3-WATCH-rs).  

  

## **API overview**

### **`CommandHandler` trait**

The same four callbacks are dispatched based on server responses:

| Method | Server byte | Meaning |
|--------|-------------|---------|
| `on_detected()` | `0x01` | Wake word detected |
| `on_thinking()` | `0x02` | Speech‑to‑text / intent processing has begun |
| `on_executed(elapsed_ms)` | `0x03` | Command was executed successfully |
| `on_failed(elapsed_ms)` | `0x04` | Command execution failed |

### **Control channels**

All internal tasks can be controlled from anywhere using global `embassy‑sync` channels:

| Channel        | Command type      | Send from anywhere …          |
|----------------|-------------------|-------------------------------|
| `VOICE_CMD`    | `VoiceCommand`    | `Enabled`, `Disabled`, `Pushed`, `Released` |
| `SPEAKER_CMD`  | `SpeakerCommand`  | `Start`, `Stop`               |
| `STREAM_CMD`   | `StreamCommand`   | `Start`, `Stop`               |

Example – enable wake‑word detection:

    let _ = yo_esp::VOICE_CMD.send(yo_esp::VoiceCommand::Enabled).await;

### **Tasks**

| Task | Description |
|------|-------------|
| `audio_capture_task(i2s_rx, stack, host, port, room, handler)` | Streams microphone audio to the server (continuous or push‑to‑talk) and dispatches `CommandHandler` callbacks. |
| `speaker_task(transfer)` | Pumps audio data from an internal ring buffer to the I²S DAC. |
| `stream_speaker(stack, listen_port)` | Accepts a TCP connection on `listen_port` and writes incoming audio into the ring buffer. |

### **Sound helpers**

Available when the `sounds` feature is enabled:

| Function | Plays |
|----------|-------|
| `play_ding()` | The “ding” notification sound |
| `play_done()` | The “done” success sound |
| `play_fail()` | The “fail” error sound |

You can also push arbitrary raw audio into the speaker pipe with `play(data: &[u8])`.

> [!NOTE]  
> **A helper script for streaming various audio types to the ESP32‑S3 is included in `examples/esp-play.sh`.**  
> **It also supports streaming your desktop microphone to the ESP32‑S3 for intercom mode.**  


### **Hardware / platform requirements**

- **ESP32‑S3 (I²S + DMA support via `esp‑hal`).**  
- **Any I²S microphone and I²S speaker codec compatible with `esp‑hal` (e.g., ES7210 + ES8311 on the official dev‑kits).**  
- **Wi‑Fi connectivity through `embassy‑net` + `esp‑radio`.**  
- **`embassy‑executor` for async tasks.**  

<br>

## **Architecture**

                  ┌──────────────────────────────────┐
                  │          yo-esp (ESP32‑S3)        │
 Microphone ──────┤ I²S RX ──► audio_capture_task     │── TCP ──► yo server (STT, TTS, intent)
                  │              ├─ wake‑word mode     │
                  │              └─ push‑to‑talk mode  │
                  │                                    │
       Speaker ◄──┤ I²S TX ◄── speaker_task           │◄─ TCP ─── (Any audio)
                  │          stream_speaker           │
                  └──────────────────────────────────┘

> * `audio_capture_task` reads I²S, converts to mono `f32`, buffers into chunks of `1280` samples, and sends them to the server.  
> * For push‑to‑talk, it sends a `PTT_START` / `PTT_DATA` / `PTT_END` sequence and receives a final success/failure byte.  
> * `stream_speaker` accepts raw PCM data over TCP and feeds it into the lock‑free pipe.  
> * `speaker_task` dequeues from that pipe and writes to the I²S TX DMA.  
> * All tasks listen on global channels and can be gracefully started/stopped at runtime.  

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
