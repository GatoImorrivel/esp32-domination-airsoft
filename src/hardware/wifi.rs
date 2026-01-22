use anyhow::Ok;
use esp_idf_svc::wifi::{AccessPointConfiguration, AsyncWifi, ClientConfiguration, EspWifi};

pub struct Wifi {
    wifi: AsyncWifi<EspWifi<'static>>,
}

impl Wifi {
    pub fn new(wifi: AsyncWifi<EspWifi<'static>>) -> Self {
        Self { wifi }
    }

    pub async fn client_mode<S: AsRef<str>>(&mut self, ssid: S, password: S) -> anyhow::Result<()> {
        self.wifi.stop().await?;

        let config = esp_idf_svc::wifi::Configuration::Client(ClientConfiguration {
            ssid: ssid.as_ref().try_into().unwrap(),
            password: password.as_ref().try_into().unwrap(),
            ..Default::default()
        });

        self.wifi.set_configuration(&config)?;

        self.wifi.start().await?;

        self.wifi.connect().await?;

        self.wifi.wait_netif_up().await?;

        Ok(())
    }

    pub async fn ap_mode(&mut self) -> anyhow::Result<()> {
        self.wifi.stop().await?;

        let config = esp_idf_svc::wifi::Configuration::AccessPoint(AccessPointConfiguration {
            ssid: "Dominacao".try_into().unwrap(),
            password: "sandidominacao".try_into().unwrap(),
            auth_method: esp_idf_svc::wifi::AuthMethod::WPA2Personal,
            ..Default::default()
        });

        self.wifi.set_configuration(&config)?;

        self.wifi.start().await?;

        Ok(())
    }
}
