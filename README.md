# **yo-esp**

[![Sponsors](https://img.shields.io/github/sponsors/QuackHack-McBlindy?logo=githubsponsors&label=Sponsor&style=flat&labelColor=ff1493&logoColor=fff&color=rgba(234,74,170,0.5) "")](https://github.com/sponsors/QuackHack-McBlindy) [![Buy Me a Coffee](https://img.shields.io/badge/Buy%20Me%20a%20Coffee-Sponsor?style=flat&logo=buymeacoffee&logoColor=fff&labelColor=ff1493&color=ff1493)](https://buymeacoffee.com/quackhackmcblindy)


`yo-esp` is a esp32 audio streaming client for the [voice assistant yo](https://github.com/QuackHack-McBlindy/yo) and is designed for bare metal and `no_std` projects. 


## **Installation**

  
Add **yo-esp** as a dependency in `Cargo.toml`.

```toml
[dependencies]
yo-esp = "0.1.0"
```
  


<br>

## **Example usage**


```bash
use yo_esp::{
    microphone::Microphone,
    run_audio_stream,
    AudioStreamConfig,
    CommandHandler,
};

struct MyHandler;
impl CommandHandler for MyHandler {
    fn on_wake_word_detected(&mut self) {
        info!("Wake word detected!");
    }
    fn on_command_executed(&mut self) {
        info!("Command executed");
    }
    fn on_command_failed(&mut self) {
        info!("Command failed");
    }
}

#[embassy_executor::task]
async fn audio_task(
    mic: Microphone,
    handler: MyHandler,
    stack: &'static embassy_net::Stack<'static>,
    addr: SocketAddr,
    config: AudioStreamConfig,
) {
    run_audio_stream(mic, handler, stack, addr, config).await;
}

// in main() after i2s init:
        let mic = Microphone::new(i2s_rx);
        let handler = MyHandler;
        let config = AudioStreamConfig {
            room: "esp", // leave as `esp`
            ..Default::default()
        };
        spawner.spawn(audio_task(mic, handler, stack, remote_addr, config)).unwrap();
```



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

**MIT**  
<br>
Contributions are welcomed.

