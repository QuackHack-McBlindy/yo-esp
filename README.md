# **yo-esp**

[![Sponsors](https://img.shields.io/github/sponsors/QuackHack-McBlindy?logo=githubsponsors&label=Sponsor&style=flat&labelColor=ff1493&logoColor=fff&color=rgba(234,74,170,0.5) "")](https://github.com/sponsors/QuackHack-McBlindy) [![Buy Me a Coffee](https://img.shields.io/badge/Buy%20Me%20a%20Coffee-Sponsor?style=flat&logo=buymeacoffee&logoColor=fff&labelColor=ff1493&color=ff1493)](https://buymeacoffee.com/quackhackmcblindy)


`yo-esp` is a esp32 audio streaming client for the [voice assistant yo](https://github.com/QuackHack-McBlindy/yo) and is designed for bare metal and `no_std` projects. 


## **Installation**

  
Add **yo-esp** as a dependency in `Cargo.toml`.

```toml
[dependencies]
yo-esp = "0.1.1"
```
  


<br>

## **Example usage**


```bash
use yo_esp::*;

struct MyHandler;
impl CommandHandler for MyHandler {
    fn on_detected(&mut self) {
        info!("Wake word detected!");
    }
    fn on_thinking(&mut self) {
        info!("server started transcription");
    }    
    fn on_executed(&mut self) {
        info!("Command executed");
    }
    fn on_failed(&mut self) {
        info!("Command failed");
    }
}


    let handler = MyHandler;

    spawner.spawn(yo_esp::audio_capture_task(
        i2s_rx, stack,
        "192.168.1.100:12345".parse().unwrap(),
        "esp",
        handler,
    )).unwrap();

    spawner.spawn(yo_esp::speaker_task(transfer)).unwrap();
    spawner.spawn(yo_esp::stream_speaker(stack, 12345)).unwrap();
}
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

