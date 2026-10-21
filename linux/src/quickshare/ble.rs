//! Waking up nearby phones. Android only advertises its Quick Share service
//! on the network while its "receive" screen is open, or for a while after
//! it hears a Bluetooth LE advertisement for service 0xFE2C with the
//! "someone nearby is sharing" payload. We broadcast that advertisement
//! through BlueZ while the user is about to share.

use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use rand::RngCore;
use zbus::names::OwnedInterfaceName;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

const SERVICE_UUID: &str = "0000fe2c-0000-1000-8000-00805f9b34fb";
const PATH: &str = "/io/github/mdhaarith/airbridge/quickshare";
const ADVERTISING_MANAGER: &str = "org.bluez.LEAdvertisingManager1";

/// The fixed prefix Android looks for, then 10 random bytes.
pub fn service_data() -> Vec<u8> {
    let mut data = vec![
        0xfc, 0x12, 0x8e, 0x01, 0x42, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    let mut salt = [0u8; 10];
    rand::thread_rng().fill_bytes(&mut salt);
    data.extend_from_slice(&salt);
    data
}

struct Advertisement {
    data: Vec<u8>,
}

#[zbus::interface(name = "org.bluez.LEAdvertisement1")]
impl Advertisement {
    /// BlueZ dropped the advertisement (adapter gone, powered off, …).
    fn release(&self) {
        tracing::debug!("Quick Share: BlueZ released the advertisement");
    }

    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        "broadcast".into()
    }

    #[zbus(property, name = "ServiceUUIDs")]
    fn service_uuids(&self) -> Vec<String> {
        vec![SERVICE_UUID.into()]
    }

    #[zbus(property, name = "ServiceData")]
    fn service_data(&self) -> HashMap<String, OwnedValue> {
        let bytes = OwnedValue::try_from(Value::from(self.data.clone())).expect("no fds in bytes");
        HashMap::from([(SERVICE_UUID.to_string(), bytes)])
    }
}

#[zbus::proxy(
    interface = "org.bluez.LEAdvertisingManager1",
    default_service = "org.bluez"
)]
trait AdvertisingManager {
    fn register_advertisement(
        &self,
        advertisement: &ObjectPath<'_>,
        options: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<()>;

    fn unregister_advertisement(&self, advertisement: &ObjectPath<'_>) -> zbus::Result<()>;
}

/// The "nearby sharing" advertisement on the first powered adapter.
pub struct Beacon {
    connection: Connection,
    adapter: OwnedObjectPath,
    active: bool,
}

impl Beacon {
    pub async fn new(connection: Connection) -> Result<Self> {
        let objects = zbus::fdo::ObjectManagerProxy::builder(&connection)
            .destination("org.bluez")?
            .path("/")?
            .build()
            .await?
            .get_managed_objects()
            .await
            .map_err(|e| anyhow!("BlueZ is not running ({e})"))?;
        let mut adapters: Vec<_> = objects
            .into_iter()
            .filter(|(_, interfaces)| interfaces.keys().any(|i| i.as_str() == ADVERTISING_MANAGER))
            .collect();
        adapters.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        if adapters.is_empty() {
            bail!("no Bluetooth adapter that can advertise");
        }
        let powered = |interfaces: &HashMap<OwnedInterfaceName, HashMap<String, OwnedValue>>| {
            interfaces
                .iter()
                .find(|(name, _)| name.as_str() == "org.bluez.Adapter1")
                .and_then(|(_, props)| props.get("Powered"))
                .and_then(|v| bool::try_from(v).ok())
                .unwrap_or(false)
        };
        let adapter = adapters
            .iter()
            .find(|(_, interfaces)| powered(interfaces))
            .map(|(path, _)| path.clone())
            .ok_or_else(|| anyhow!("Bluetooth is turned off"))?;
        Ok(Self {
            connection,
            adapter,
            active: false,
        })
    }

    async fn manager(&self) -> zbus::Result<AdvertisingManagerProxy<'_>> {
        AdvertisingManagerProxy::builder(&self.connection)
            .path(self.adapter.clone())?
            .build()
            .await
    }

    pub async fn start(&mut self) -> Result<()> {
        if self.active {
            return Ok(());
        }
        let path = ObjectPath::try_from(PATH)?;
        let server = self.connection.object_server();
        server.remove::<Advertisement, _>(&path).await.ok();
        server
            .at(
                &path,
                Advertisement {
                    data: service_data(),
                },
            )
            .await?;
        self.manager()
            .await?
            .register_advertisement(&path, HashMap::new())
            .await?;
        self.active = true;
        tracing::debug!("Quick Share: telling nearby phones we want to share");
        Ok(())
    }

    pub async fn stop(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        let path = ObjectPath::from_static_str_unchecked(PATH);
        if let Ok(manager) = self.manager().await {
            let _ = manager.unregister_advertisement(&path).await;
        }
        let _ = self
            .connection
            .object_server()
            .remove::<Advertisement, _>(&path)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::sync::{Arc, Mutex};

    use zbus::message::Header;

    use super::*;

    type Seen = Arc<Mutex<Vec<HashMap<String, OwnedValue>>>>;

    struct FakeManager {
        seen: Seen,
    }

    #[zbus::interface(name = "org.bluez.LEAdvertisingManager1")]
    impl FakeManager {
        /// Like BlueZ: reads the advertisement back from the caller.
        async fn register_advertisement(
            &self,
            advertisement: OwnedObjectPath,
            _options: HashMap<String, OwnedValue>,
            #[zbus(header)] header: Header<'_>,
            #[zbus(connection)] connection: &Connection,
        ) -> zbus::fdo::Result<()> {
            let sender = header.sender().unwrap().to_owned();
            let props = zbus::fdo::PropertiesProxy::builder(connection)
                .destination(sender)?
                .path(advertisement)?
                .build()
                .await?
                .get_all(zbus::names::InterfaceName::from_static_str_unchecked(
                    "org.bluez.LEAdvertisement1",
                ))
                .await?;
            self.seen.lock().unwrap().push(props);
            Ok(())
        }

        fn unregister_advertisement(&self, _advertisement: OwnedObjectPath) {}
    }

    struct FakeAdapter;

    #[zbus::interface(name = "org.bluez.Adapter1")]
    impl FakeAdapter {
        #[zbus(property)]
        fn powered(&self) -> bool {
            true
        }
    }

    /// Runs against a fake BlueZ on a private bus, if dbus-daemon exists.
    #[tokio::test]
    async fn advertises_the_nearby_sharing_payload() {
        let Ok(mut daemon) = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .spawn()
        else {
            eprintln!("dbus-daemon not installed; skipping");
            return;
        };
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim().to_string();

        let seen: Seen = Default::default();
        let bluez = zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .name("org.bluez")
            .unwrap()
            .serve_at("/", zbus::fdo::ObjectManager)
            .unwrap()
            .serve_at("/org/bluez/hci0", FakeManager { seen: seen.clone() })
            .unwrap()
            .serve_at("/org/bluez/hci0", FakeAdapter)
            .unwrap()
            .build()
            .await
            .unwrap();

        let app = zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap();
        let mut beacon = Beacon::new(app).await.unwrap();
        assert_eq!(beacon.adapter.as_str(), "/org/bluez/hci0");
        beacon.start().await.unwrap();
        beacon.start().await.unwrap(); // idempotent
        beacon.stop().await;
        beacon.start().await.unwrap();
        beacon.stop().await;

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        let props = &seen[0];
        assert_eq!(
            String::try_from(props["Type"].clone()).unwrap(),
            "broadcast"
        );
        let uuids: Vec<String> = props["ServiceUUIDs"].clone().try_into().unwrap();
        assert_eq!(uuids, [SERVICE_UUID]);
        let data: HashMap<String, OwnedValue> = props["ServiceData"].clone().try_into().unwrap();
        let bytes: Vec<u8> = data[SERVICE_UUID].clone().try_into().unwrap();
        assert_eq!(bytes.len(), 24);
        assert_eq!(&bytes[..5], &[0xfc, 0x12, 0x8e, 0x01, 0x42]);
        drop(bluez);
        let _ = daemon.kill();
        let _ = daemon.wait();
    }
}
