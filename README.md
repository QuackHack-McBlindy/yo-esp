# **yo-esp**

[![Sponsors](https://img.shields.io/github/sponsors/QuackHack-McBlindy?logo=githubsponsors&label=Sponsor&style=flat&labelColor=ff1493&logoColor=fff&color=rgba(234,74,170,0.5) "")](https://github.com/sponsors/QuackHack-McBlindy) [![Buy Me a Coffee](https://img.shields.io/badge/Buy%20Me%20a%20Coffee-Sponsor?style=flat&logo=buymeacoffee&logoColor=fff&labelColor=ff1493&color=ff1493)](https://buymeacoffee.com/quackhackmcblindy)


`yo-esp` is a esp32s3 audio streaming client for the [voice assistant yo](https://github.com/QuackHack-McBlindy/yo) and is designed for bare metal and `no_std` projects. 


## **Installation**

  
Add **yo-esp** as a dependency in `Cargo.toml`.

```toml
[dependencies]
yo-esp = "0.1.2"
```

You will also need a network stack `embassy-net`    


<br>

## **Example usage**


```bash
use yo_esp::{audio_capture_task, speaker_task, stream_speaker, CommandHandler, play_ding, play_done, play_fail};

// CREATE A CALLBACK HANDLER
struct VoiceHandler;

// DEFINE ACTIONS WHEN SERVER SENDS BYTES
impl yo_esp::CommandHandler for VoiceHandler {    
    // 0x01 == WAKE WORD DETECTED
    fn on_detected(&mut self) -> core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = ()> + '_>> {
        alloc::boxed::Box::pin(async {
            info!("💥 DETECTED Wake Word!");
            // PLAY DING SOUND
            yo_esp::play_ding().await;
            // AND TURN ON DISPLAY
            crate::components::display::brightness_set("70");      
        })
    }

    // 0x02 == SERVER STARTED TRANSCRIPTION
    fn on_thinking(&mut self) -> core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = ()> + '_>> {
        Box::pin(async {
            info!("🧠 THINKING..."); 
        })
    }

    // 0x03 == COMMAND EXECUTED
    fn on_executed(&mut self, ms: Option<u64>) -> core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = ()> + '_>> {
        Box::pin(async move {       
            info!("✅ Executed command!");        
            // PLAY DONE SOUND
            yo_esp::play_done().await;
            // AND TURN OFF DISPLAY
            crate::components::display::brightness_set("0");
        })
    }

    // 0x04 == FAILED COMMAND EXECUTION
    fn on_failed(&mut self, ms: Option<u64>) -> core::pin::Pin<alloc::boxed::Box<dyn core::future::Future<Output = ()> + '_>> {
        Box::pin(async move {         
            info!("💩 FAILED execution!");
            // PLAY FAIL SOUND
            yo_esp::play_fail().await;
           // AND TURN OFF DISPLAY
           crate::components::display::brightness_set("0");
        })
    }
}

extern crate alloc;
use alloc::boxed::Box;

// BOOTLOADER
esp_bootloader_esp_idf::esp_app_desc!();

// COMPILE-TIME ENVIORMENT VARIABLES
const SSID: &str = env!("WIFI_SSID");
const PASSWORD: &str = env!("WIFI_PASSWORD");
const BACKEND_TCP_HOST: &str = env!("BACKEND_TCP_HOST");

const SAMPLE_RATE: u32 = 16000;
const SAMPLE_COUNT: usize = 256;
const BUFFER_SIZE: usize = 4 * 4092;



pub static ES7210: CsMutex<RefCell<Option<components::es7210::Es7210>>> = CsMutex::new(RefCell::new(None));
pub static ES8311: CsMutex<RefCell<Option<components::es8311::Es8311>>> = CsMutex::new(RefCell::new(None));
pub static I2C_BUS: CsMutex<RefCell<Option<I2cBus>>> = CsMutex::new(RefCell::new(None));
pub type I2cBus = I2c<'static, Blocking>;

// MAIN
#[allow(clippy::large_stack_frames)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // ...
    
    // I2C BUS A
    let mut i2c_a = I2c::new(
        peripherals.I2C0,
        I2cConfig::default().with_frequency(Rate::from_khz(100)),
    )
    .unwrap()
    .with_sda(peripherals.GPIO8)
    .with_scl(peripherals.GPIO18);    

    // I2C BUS B
    let mut i2c_b = I2c::new(
        peripherals.I2C1,
        I2cConfig::default().with_frequency(Rate::from_khz(50)),
    )
    .unwrap()
    .with_sda(peripherals.GPIO41)
    .with_scl(peripherals.GPIO40);

    // LOCK & SHARE BUSSES
    let i2c_a_mutex = Box::leak(Box::new(CsMutex::new(RefCell::new(i2c_a))));
    let i2c_b_mutex = Box::leak(Box::new(CsMutex::new(RefCell::new(i2c_b))));

    // AUDIO CODEC CONFIGURATION
    let es7210 = es7210_rs::es7210::Es7210::new(0x40);
    let es8311 = es8311_rs::es8311::Es8311::new(0x18);

    { // configure audio codecs
        let mut i2c = CriticalSectionDevice::new(&i2c_a_mutex);

        // ES7210 (ADC)
        let codec_cfg = components::es7210::CodecConfig {
            sample_rate_hz: 16000,
            mclk_ratio: 256,
            i2s_format: es7210_rs::es7210::I2sFormat::I2S,
            bit_width: es7210_rs::es7210::I2sBits::Bits16,
            mic_bias: es7210_rs::es7210::MicBias::V2_87,
            mic_gain: es7210_rs::es7210::MicGain::Gain30dB,
            tdm_enable: false,
        };
        match es7210.config_codec(&mut i2c, &codec_cfg) {
            Ok(()) => info!("ES7210 initialized successfully"),
            Err(e) => info!("ES7210 init failed: {:?}", Debug2Format(&e)),
        }
        if let Err(e) = es7210.gain_set(&mut i2c, 20) {
            info!("ES7210 volume set failed: {:?}", Debug2Format(&e));
        }
        if let Err(e) = es7210.set_mute(&mut i2c, false) {
            info!("Failed to configure ES7210 mute status {:?}", Debug2Format(&e));
        }
        
        // ES8311 (DAC)
        let clock_cfg = es8311_rs::es8311::ClockConfig {
            mclk_inverted: false,
            sclk_inverted: false,
            mclk_from_mclk_pin: true,
            mclk_frequency: 4096000,
            sample_frequency: 16000,
        };
        match es8311.init(
            &mut i2c,
            &clock_cfg,
            es8311_rs::es8311::Resolution::Bits16,
            es8311_rs::es8311::Resolution::Bits16,
        ) {
            Ok(()) => info!("ES8311 initialised successfully"),
            Err(e) => info!("ES8311 init failed: {:?}", Debug2Format(&e)),
        }
        let _ = es8311.volume_set(&mut i2c, 80, None);
        let _ = es8311.mute(&mut i2c, false);
    } // RELEASE I2C

    // WIFI SETUP
    let (mut wifi_controller, interfaces) = esp_radio::wifi::new(
        peripherals.WIFI,
        ControllerConfig::default(),
    )
    .expect("Wi-Fi - ❌ init failed");

    let station_config = StationConfig::default()
        .with_ssid(SSID)
        .with_password(PASSWORD.to_string());

    let wifi_config = WifiConfig::Station(station_config);
    wifi_controller.set_config(&wifi_config).unwrap();
    let station = interfaces.station;

    spawn!(spawner, base::wifi::connection(wifi_controller));
    
    // EMBASSY-NET SETUP
    let net_config = NetConfig::dhcpv4(DhcpConfig::default());
    let rng = Rng::new();
    let seed = (u64::from(rng.random())) << 32 | u64::from(rng.random());
    
    let stack_resources = mk_static!(StackResources<16>, StackResources::<16>::new());
    
    let (stack, runner) = embassy_net::new(
        station,
        net_config,
        stack_resources,
        seed,
    );
    let stack = mk_static!(Stack<'static>, stack);
    
    spawn!(spawner, base::wifi::net_task(runner));
    
    stack.wait_link_up().await;
    stack.wait_config_up().await;
    
    let ip = loop {
        if let Some(config) = stack.config_v4() {
            break config.address;
        }
        Timer::after(Duration::from_millis(500)).await;
    };
    let ip_addr = ip.address();
    let ip_raw = u32::from(ip_addr);
    info!("IP: {}", ip_addr);
    
    // RESOLVE BACKEND ADDRESS
    let BACKEND_TCP_PORT: u16 = env!("BACKEND_TCP_PORT").parse().expect("Invalid port");
    let remote_addr = loop {
        match stack.dns_query(BACKEND_TCP_HOST, DnsQueryType::A).await {
            Ok(addr) => break (addr[0], BACKEND_TCP_PORT).into(),
            Err(e) => {
                info!("DNS lookup error for {}: {}", BACKEND_TCP_HOST, e);
                Timer::after(Duration::from_secs(5)).await;
            }
        }
    };
    
     
    // I2S AUDIO SETUP 
    let (rx_buffer, rx_descriptors, tx_buffer, tx_descriptors) = dma_buffers!(BUFFER_SIZE);

    let i2s = I2s::new(
        peripherals.I2S0,
        peripherals.DMA_CH0,
        esp_hal::i2s::master::Config::new_tdm_philips()
            .with_signal_loopback(true)
            .with_sample_rate(Rate::from_hz(16000))
            .with_data_format(esp_hal::i2s::master::DataFormat::Data16Channel16)
            .with_endianness(esp_hal::i2s::master::Endianness::LittleEndian) 
            .with_channels(esp_hal::i2s::master::Channels::STEREO),            
    )
    .unwrap()
    .into_async()
    .with_mclk(peripherals.GPIO2);

    // AUDIO OUTPUT
    // BUILD I2S TX (MASTER) WITH BCLK, LRCLK AND DIGITAL OUT PINS 
    let i2s_tx = i2s.i2s_tx
        .with_bclk(peripherals.GPIO17)
        .with_ws(peripherals.GPIO45)
        .with_dout(peripherals.GPIO15)
        .build(tx_descriptors);

    // AUDIO INPUT
    // BUILD I2S RX (SLAVE) WITH DIGITAL-IN PIN 
    let i2s_rx = i2s
        .i2s_rx
        .with_din(peripherals.GPIO16)
        .build(rx_descriptors);

    // I2S TX CIRCULAR WRITE
    // CONTINUOSLY WRITE TO I2S TX TO KEEP CLOCKS UP FOR RX (SLAVE)
    let tx_transfer = match i2s_tx.write_dma_circular_async(tx_buffer) {
        Ok(t) => t,
        Err(e) => {
            error!("I2S circular TX failed: {:?}", Debug2Format(&e));
            panic!("I2S setup error");
        }
    };
    
    // YO-HANDLER 
    let handler: alloc::boxed::Box<dyn yo_esp::CommandHandler> = alloc::boxed::Box::new(VoiceHandler);  

    // START SPEAKER
    spawn!(spawner, yo_esp::speaker_task(tx_transfer));
    
    // SERVER -> ESP32
    // SPEAKER SERVER TASK (STREAM AUDIO TO THE SPEAKER OVER TCP PORT 12345)
    spawn!(spawner, yo_esp::stream_speaker(stack, BACKEND_TCP_PORT)); 
    
    // ESP32 -> SERVER
    // MICROPHONE TASK (STREAM MICROPHONE OVER TCP PORT 12345)
    spawn!(spawner, yo_esp::audio_capture_task(i2s_rx, stack, remote_addr, "esp", handler));
    
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
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
