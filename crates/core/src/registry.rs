use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{Connection, ConnectionConfig, Driver, DriverInfo, Error, Result};

/// The set of drivers compiled into this build, keyed by [`DriverInfo::id`].
#[derive(Default, Clone)]
pub struct DriverRegistry {
    drivers: BTreeMap<&'static str, Arc<dyn Driver>>,
}

impl DriverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, driver: impl Driver + 'static) {
        self.drivers.insert(driver.info().id, Arc::new(driver));
    }

    pub fn list(&self) -> Vec<DriverInfo> {
        self.drivers.values().map(|d| d.info()).collect()
    }

    pub fn info(&self, driver_id: &str) -> Option<DriverInfo> {
        self.drivers.get(driver_id).map(|d| d.info())
    }

    pub async fn connect(&self, config: &ConnectionConfig) -> Result<Box<dyn Connection>> {
        let driver = self
            .drivers
            .get(config.driver.as_str())
            .ok_or_else(|| Error::UnknownDriver(config.driver.clone()))?;
        driver.connect(config).await
    }
}
