//! Waking up nearby phones on Windows: the same Bluetooth LE "nearby sharing"
//! advertisement as on Linux (see ble.rs), sent with WinRT's advertisement
//! publisher.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use windows::Devices::Bluetooth::Advertisement::{
    BluetoothLEAdvertisementDataSection, BluetoothLEAdvertisementPublisher,
    BluetoothLEAdvertisementPublisherStatus,
};
use windows::Devices::Bluetooth::BluetoothAdapter;
use windows::Devices::Radios::RadioState;
use windows::Storage::Streams::DataWriter;

/// "Service Data - 16-bit UUID" advertising data type.
const SERVICE_DATA_16: u8 = 0x16;

pub struct Beacon {
    publisher: Option<BluetoothLEAdvertisementPublisher>,
}

impl Beacon {
    /// Fails with the reason when this computer cannot advertise.
    pub async fn new() -> Result<Self> {
        tokio::task::spawn_blocking(check_adapter).await??;
        Ok(Self { publisher: None })
    }

    pub async fn start(&mut self) -> Result<()> {
        if self.publisher.is_none() {
            self.publisher = Some(tokio::task::spawn_blocking(publish).await??);
            tracing::debug!("Quick Share: telling nearby phones we want to share");
        }
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(publisher) = self.publisher.take() {
            let _ = publisher.Stop();
        }
    }
}

fn check_adapter() -> Result<()> {
    let adapter = BluetoothAdapter::GetDefaultAsync()
        .and_then(|operation| operation.get())
        .map_err(|_| anyhow!("this computer has no Bluetooth"))?;
    if !adapter.IsLowEnergySupported()? || !adapter.IsPeripheralRoleSupported()? {
        bail!("this computer's Bluetooth cannot advertise to phones");
    }
    let radio = adapter.GetRadioAsync()?.get()?;
    if radio.State()? != RadioState::On {
        bail!("Bluetooth is turned off");
    }
    Ok(())
}

fn publish() -> Result<BluetoothLEAdvertisementPublisher> {
    // The 16-bit service UUID 0xFE2C (little-endian), then its data.
    let mut data = vec![0x2c, 0xfe];
    data.extend(super::service_data());
    let writer = DataWriter::new()?;
    writer.WriteBytes(&data)?;
    let section =
        BluetoothLEAdvertisementDataSection::Create(SERVICE_DATA_16, &writer.DetachBuffer()?)?;
    let publisher = BluetoothLEAdvertisementPublisher::new()?;
    publisher
        .Advertisement()?
        .DataSections()?
        .Append(&section)?;
    publisher.Start()?;
    // Windows reports a refusal (radio off, blocked by policy) through the status.
    for _ in 0..20 {
        match publisher.Status()? {
            BluetoothLEAdvertisementPublisherStatus::Started => break,
            BluetoothLEAdvertisementPublisherStatus::Aborted => {
                bail!("Windows refused the Bluetooth advertisement (Bluetooth off or blocked by policy)")
            }
            _ => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    Ok(publisher)
}
