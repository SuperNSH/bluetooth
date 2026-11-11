//! `Server`: the peripheral role, with one advertisement at a time over a backend.

use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::broadcast;

use crate::backend::{AdvertiseOptions, GattRequest, ServerBackend, Service, Uuid};
use crate::error::{BluetoothError, ErrorKind};

/// One advertisement per `Server`. `active` names the generation that owns the slot, so a stop
/// or a stale start of an older call cannot clear a newer claim.
#[derive(Default)]
struct AdvertiseGate {
    next: u64,
    active: Option<u64>,
}

/// The peripheral role: serve GATT services and advertise them.
///
/// One advertisement runs per `Server` at a time; a second gets `ERR_BLUETOOTH_IN_PROGRESS`.
pub struct Server {
    backend: Arc<dyn ServerBackend>,
    advertising: Mutex<AdvertiseGate>,
}

impl Server {
    /// A server over `backend`. Tests and fakes use this.
    pub fn new(backend: Arc<dyn ServerBackend>) -> Self {
        Self {
            backend,
            advertising: Mutex::new(AdvertiseGate::default()),
        }
    }

    fn gate(&self) -> std::sync::MutexGuard<'_, AdvertiseGate> {
        self.advertising
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The target's server, or `ERR_BLUETOOTH_UNSUPPORTED` when the platform has no server
    /// backend, the web included.
    pub fn platform_server() -> Result<Self, BluetoothError> {
        match crate::platform::server_backend() {
            Some(backend) => Ok(Self::new(backend)),
            None => Err(BluetoothError::new(ErrorKind::Unsupported)),
        }
    }

    /// Serves `service`. A duplicate service UUID fails with `ERR_BLUETOOTH_SERVER_FAILED`.
    pub async fn add_service(&self, service: Service) -> Result<(), BluetoothError> {
        self.backend.add_service(service).await
    }

    /// Stops serving the service. Ok when no such service is served.
    pub async fn remove_service(&self, uuid: Uuid) -> Result<(), BluetoothError> {
        self.backend.remove_service(uuid).await
    }

    /// Starts advertising. One advertisement runs at a time; a second gets
    /// `ERR_BLUETOOTH_IN_PROGRESS`. Dropping the call while the backend starts rolls the claim
    /// back; a start that completes after a stop re-asserts the claim, mirroring the backend.
    pub async fn advertise(&self, options: AdvertiseOptions) -> Result<(), BluetoothError> {
        let generation = {
            let mut gate = self.gate();
            if gate.active.is_some() {
                return Err(BluetoothError::new(ErrorKind::InProgress));
            }
            // Claim before the await, so two concurrent calls cannot both start.
            let generation = gate.next;
            gate.next += 1;
            gate.active = Some(generation);
            generation
        };
        let mut slot = AdvertiseSlot {
            gate: &self.advertising,
            generation,
            armed: true,
        };
        match self.backend.start_advertising(options).await {
            Ok(()) => {
                slot.keep();
                Ok(())
            }
            // The slot's drop rolls the claim back.
            Err(error) => Err(error),
        }
    }

    /// Stops advertising. Ok when not advertising.
    pub async fn stop_advertising(&self) -> Result<(), BluetoothError> {
        let result = self.backend.stop_advertising().await;
        if result.is_ok() {
            self.gate().active = None;
        }
        result
    }

    /// Whether the server is advertising.
    pub fn is_advertising(&self) -> bool {
        self.gate().active.is_some()
    }

    /// Sets a served characteristic's value, for the centrals that read it.
    pub async fn set_characteristic_value(
        &self,
        service: Uuid,
        characteristic: Uuid,
        value: Vec<u8>,
    ) -> Result<(), BluetoothError> {
        self.backend
            .set_characteristic_value(service, characteristic, value)
            .await
    }

    /// Pushes a value to the subscribed centrals. An unknown handle fails with
    /// `ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND` or `ERR_BLUETOOTH_SERVICE_NOT_FOUND`, and no
    /// subscriber with `ERR_BLUETOOTH_CHARACTERISTIC_NOTIFY_CHANGE_FAILED`.
    pub async fn notify(
        &self,
        service: Uuid,
        characteristic: Uuid,
        data: &[u8],
    ) -> Result<(), BluetoothError> {
        self.backend.notify(service, characteristic, data).await
    }

    /// The requests the centrals send, from the next one on.
    pub async fn requests(&self) -> Result<broadcast::Receiver<GattRequest>, BluetoothError> {
        self.backend.requests().await
    }
}

/// Holds one advertise start's claim. A start that is dropped or refused rolls its own
/// generation back; a start that completes after a stop re-asserts the claim, because the
/// backend is advertising again and no newer claim may be clobbered.
struct AdvertiseSlot<'a> {
    gate: &'a Mutex<AdvertiseGate>,
    generation: u64,
    armed: bool,
}

impl AdvertiseSlot<'_> {
    /// Adopts the completed start: keep the claim, filling the hole a stop left.
    fn keep(&mut self) {
        self.armed = false;
        let mut gate = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        if gate.active.is_none() {
            gate.active = Some(self.generation);
        }
    }
}

impl Drop for AdvertiseSlot<'_> {
    fn drop(&mut self) {
        if self.armed {
            let mut gate = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
            if gate.active == Some(self.generation) {
                gate.active = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{
        Characteristic, CharacteristicProperties, GattRequestKind, Service as ServiceType,
    };
    use crate::error::ErrorKind as Kind;
    use crate::fake::{FakeServerBackend, ServerOp};
    use crate::unsupported::UnsupportedServer;
    use std::future::ready;
    use std::time::Duration;

    fn heart_rate_service() -> ServiceType {
        ServiceType {
            uuid: Uuid::parse("180D").unwrap(),
            primary: true,
            characteristics: vec![Characteristic {
                uuid: Uuid::parse("2A37").unwrap(),
                service_uuid: Uuid::parse("180D").unwrap(),
                properties: CharacteristicProperties::NOTIFY,
                descriptors: Vec::new(),
                value: None,
            }],
            included_services: Vec::new(),
        }
    }

    async fn served() -> (Arc<FakeServerBackend>, Server) {
        let fake = FakeServerBackend::new();
        let server = Server::new(fake.clone());
        server.add_service(heart_rate_service()).await.unwrap();
        (fake, server)
    }

    #[tokio::test]
    async fn services_are_added_once_and_removed_idempotently() {
        let fake = FakeServerBackend::new();
        let server = Server::new(fake);
        server.add_service(heart_rate_service()).await.unwrap();
        let error = server.add_service(heart_rate_service()).await.unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_SERVER_FAILED");
        server
            .remove_service(Uuid::parse("180D").unwrap())
            .await
            .unwrap();
        server
            .remove_service(Uuid::parse("180D").unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn one_advertisement_at_a_time() {
        let (_fake, server) = served().await;
        assert!(!server.is_advertising());
        server.advertise(AdvertiseOptions::default()).await.unwrap();
        assert!(server.is_advertising());
        let error = server
            .advertise(AdvertiseOptions::default())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_IN_PROGRESS");
        server.stop_advertising().await.unwrap();
        server.stop_advertising().await.unwrap();
        assert!(!server.is_advertising());
    }

    #[tokio::test]
    async fn a_failed_advertise_frees_the_slot() {
        let (fake, server) = served().await;
        fake.fail_next_server(
            ServerOp::StartAdvertising,
            BluetoothError::new(Kind::AdvertiseFailed),
        );
        let error = server
            .advertise(AdvertiseOptions::default())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_ADVERTISE_FAILED");
        assert!(!server.is_advertising());
        server.advertise(AdvertiseOptions::default()).await.unwrap();
    }

    #[tokio::test]
    async fn values_and_notifications_carry_their_codes() {
        let (fake, server) = served().await;
        let service = Uuid::parse("180D").unwrap();
        let measurement = Uuid::parse("2A37").unwrap();
        server
            .set_characteristic_value(service.clone(), measurement.clone(), vec![1, 2])
            .await
            .unwrap();
        assert_eq!(
            fake.captured_values(),
            vec![(service.clone(), measurement.clone(), vec![1, 2])]
        );
        // 2A37 is notify-capable, so the simulated central takes the notify.
        server
            .notify(service.clone(), measurement.clone(), &[1])
            .await
            .unwrap();
        let error = server
            .notify(Uuid::parse("180F").unwrap(), measurement, &[1])
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_SERVICE_NOT_FOUND");
        let error = server
            .notify(service, Uuid::parse("FFFF").unwrap(), &[1])
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_CHARACTERISTIC_NOT_FOUND");
    }

    #[tokio::test]
    async fn requests_reach_the_server() {
        let (fake, server) = served().await;
        let mut requests = server.requests().await.unwrap();
        let request = GattRequest {
            service_uuid: Uuid::parse("180D").unwrap(),
            characteristic_uuid: Uuid::parse("2A37").unwrap(),
            kind: GattRequestKind::Read { offset: 0 },
        };
        fake.inject_request(request);
        assert!(requests.recv().await.is_ok());
    }

    #[tokio::test]
    async fn an_unsupported_platform_has_no_server() {
        // No platform serves a server backend yet (the web bridge is central-only); whichever
        // area lands one owns this expectation from then on.
        if let Err(error) = Server::platform_server() {
            assert_eq!(error.code(), "ERR_BLUETOOTH_UNSUPPORTED");
        }
        let server = Server::new(Arc::new(UnsupportedServer));
        let error = server.add_service(heart_rate_service()).await.unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_UNSUPPORTED");
        let error = server
            .advertise(AdvertiseOptions::default())
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_BLUETOOTH_UNSUPPORTED");
        assert!(!server.is_advertising());
    }

    /// A server backend whose start_advertising waits on a gate, to hold a call mid-start.
    struct GatedServer {
        start_gate: tokio::sync::watch::Receiver<bool>,
    }

    impl crate::backend::ServerBackend for GatedServer {
        fn add_service(
            &self,
            _service: crate::backend::Service,
        ) -> crate::backend::BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(ready(Ok(())))
        }
        fn remove_service(
            &self,
            _uuid: Uuid,
        ) -> crate::backend::BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(ready(Ok(())))
        }
        fn start_advertising(
            &self,
            _options: AdvertiseOptions,
        ) -> crate::backend::BackendFuture<'_, Result<(), BluetoothError>> {
            let mut gate = self.start_gate.clone();
            Box::pin(async move {
                while !*gate.borrow() {
                    if gate.changed().await.is_err() {
                        return Err(BluetoothError::new(Kind::Backend));
                    }
                }
                Ok(())
            })
        }
        fn stop_advertising(
            &self,
        ) -> crate::backend::BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(ready(Ok(())))
        }
        fn is_advertising(
            &self,
        ) -> crate::backend::BackendFuture<'_, Result<bool, BluetoothError>> {
            Box::pin(ready(Ok(false)))
        }
        fn set_characteristic_value(
            &self,
            _service: Uuid,
            _characteristic: Uuid,
            _value: Vec<u8>,
        ) -> crate::backend::BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(ready(Ok(())))
        }
        fn notify(
            &self,
            _service: Uuid,
            _characteristic: Uuid,
            _data: &[u8],
        ) -> crate::backend::BackendFuture<'_, Result<(), BluetoothError>> {
            Box::pin(ready(Ok(())))
        }
        fn requests(
            &self,
        ) -> crate::backend::BackendFuture<
            '_,
            Result<broadcast::Receiver<GattRequest>, BluetoothError>,
        > {
            Box::pin(ready(Ok(broadcast::channel(32).1)))
        }
    }

    #[tokio::test]
    async fn a_dropped_advertise_frees_the_slot() {
        let (_gate, gate_rx) = tokio::sync::watch::channel(false);
        let server = Server::new(Arc::new(GatedServer {
            start_gate: gate_rx,
        }));
        {
            let mut call = std::pin::pin!(server.advertise(AdvertiseOptions::default()));
            // The slot is claimed, the backend never answers, and the call is abandoned.
            let _ = tokio::time::timeout(Duration::from_millis(25), call.as_mut()).await;
        }
        assert!(!server.is_advertising());
        match tokio::time::timeout(
            Duration::from_millis(25),
            server.advertise(AdvertiseOptions::default()),
        )
        .await
        {
            // Still starting: the second call passed the slot and waits on the backend.
            Err(_still_starting) => {}
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                assert_ne!(error.code(), "ERR_BLUETOOTH_IN_PROGRESS", "the slot leaked")
            }
        }
    }

    #[tokio::test]
    async fn a_late_advertise_start_reasserts_the_claim() {
        let (gate_tx, gate_rx) = tokio::sync::watch::channel(false);
        let server = Server::new(Arc::new(GatedServer {
            start_gate: gate_rx,
        }));
        let mut call = std::pin::pin!(server.advertise(AdvertiseOptions::default()));
        // The claim is made; the backend start waits on the gate.
        let _ = tokio::time::timeout(Duration::from_millis(25), call.as_mut()).await;
        assert!(server.is_advertising());
        // A stop lands while the start is still in flight, then the start succeeds after it.
        server.stop_advertising().await.unwrap();
        assert!(!server.is_advertising());
        gate_tx.send(true).unwrap();
        call.await.unwrap();
        assert!(
            server.is_advertising(),
            "a start that completes after a stop must re-assert the claim"
        );
    }
}
