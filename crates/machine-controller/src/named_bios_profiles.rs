/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use bmc_vendor::BMCVendor;
use db::DatabaseResult;
use libredfish::model::service_root::RedfishVendor;
use model::machine_bios::BiosProfile;
use sqlx::PgPool;

use crate::config::NamedBiosProfile;

pub const NAMED_BIOS_PROFILE_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq)]
pub struct StoredBiosProfile {
    pub profile: NamedBiosProfile,
    pub revision: String,
}

pub type StoredBiosProfiles = HashMap<String, StoredBiosProfile>;

impl TryFrom<&BiosProfile> for StoredBiosProfile {
    type Error = String;

    fn try_from(profile: &BiosProfile) -> Result<Self, Self::Error> {
        let vendor = redfish_vendor(&profile.vendor).ok_or_else(|| {
            format!(
                "BIOS profile vendor {} is not a BMC vendor NICo can match a host to",
                profile.vendor
            )
        })?;
        Ok(Self {
            profile: NamedBiosProfile {
                vendor,
                model: libredfish::model_coerce(profile.model.trim()),
                attributes: profile.attributes.clone().into_iter().collect(),
            },
            revision: profile.revision.clone(),
        })
    }
}

fn redfish_vendor(vendor: &str) -> Option<RedfishVendor> {
    let vendor = vendor.trim();
    if let Ok(redfish_vendor) =
        serde_json::from_value::<RedfishVendor>(serde_json::Value::String(vendor.to_string()))
    {
        return (redfish_vendor != RedfishVendor::Unknown).then_some(redfish_vendor);
    }
    let bmc_vendor = match BMCVendor::from_udev_dmi(vendor) {
        BMCVendor::Unknown => BMCVendor::from(vendor),
        known => known,
    };
    match bmc_vendor {
        BMCVendor::Lenovo => Some(RedfishVendor::Lenovo),
        BMCVendor::LenovoAMI => Some(RedfishVendor::LenovoAMI),
        BMCVendor::Dell => Some(RedfishVendor::Dell),
        BMCVendor::Supermicro => Some(RedfishVendor::Supermicro),
        BMCVendor::Hpe => Some(RedfishVendor::Hpe),
        BMCVendor::Liteon => Some(RedfishVendor::LiteOnPowerShelf),
        BMCVendor::Delta => Some(RedfishVendor::DeltaPowerShelf),
        BMCVendor::Sushy => Some(RedfishVendor::Sushy),
        BMCVendor::Nvidia | BMCVendor::Unknown => None,
    }
}

pub fn stored_bios_profiles(profiles: &[BiosProfile]) -> StoredBiosProfiles {
    profiles
        .iter()
        .filter_map(|profile| match StoredBiosProfile::try_from(profile) {
            Ok(stored) => Some((profile.name.clone(), stored)),
            Err(error) => {
                tracing::warn!(bios_profile = %profile.name, %error, "Ignoring a stored BIOS profile");
                None
            }
        })
        .collect()
}

#[derive(Clone)]
pub struct NamedBiosProfileCache {
    inner: Arc<Mutex<CachedBiosProfiles>>,
    ttl: Duration,
}

#[derive(Default)]
struct CachedBiosProfiles {
    generation: u64,
    loaded_at: Option<Instant>,
    profiles: Arc<StoredBiosProfiles>,
}

impl Default for NamedBiosProfileCache {
    fn default() -> Self {
        Self::new(NAMED_BIOS_PROFILE_REFRESH_INTERVAL)
    }
}

impl NamedBiosProfileCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Default::default(),
            ttl,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_profiles(profiles: StoredBiosProfiles) -> Self {
        Self {
            inner: Arc::new(Mutex::new(CachedBiosProfiles {
                generation: 0,
                loaded_at: Some(Instant::now()),
                profiles: Arc::new(profiles),
            })),
            ttl: Duration::MAX,
        }
    }

    pub fn invalidate(&self) {
        let mut cache = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        cache.generation += 1;
        cache.loaded_at = None;
    }

    pub async fn profiles(&self, db_pool: &PgPool) -> DatabaseResult<Arc<StoredBiosProfiles>> {
        let generation = {
            let cache = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            if cache
                .loaded_at
                .is_some_and(|loaded_at| loaded_at.elapsed() < self.ttl)
            {
                return Ok(cache.profiles.clone());
            }
            cache.generation
        };
        let loaded_at = Instant::now();
        let profiles = Arc::new(stored_bios_profiles(
            &db::named_bios_profile::list(db_pool).await?,
        ));
        let mut cache = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.generation == generation {
            cache.loaded_at = Some(loaded_at);
            cache.profiles = profiles.clone();
        }
        Ok(profiles)
    }
}

#[cfg(test)]
mod tests {
    use carbide_test_support::value_scenarios;
    use serde_json::json;

    use super::*;

    fn profile(vendor: &str, model: &str) -> BiosProfile {
        BiosProfile {
            name: "lenovo-h200".to_string(),
            vendor: vendor.to_string(),
            model: model.to_string(),
            attribute_registry: None,
            attributes: json!({"OperatingModes_ChooseOperatingMode": "MaximumPerformance"})
                .as_object()
                .unwrap()
                .clone(),
            revision: "3f2a9c1d7b4e".to_string(),
            updated_at: None,
        }
    }

    #[test]
    fn stored_profiles_name_the_host_the_way_the_gate_reads_it() {
        value_scenarios!(
            run = |(vendor, model): (&str, &str)| {
                StoredBiosProfile::try_from(&profile(vendor, model))
                    .map(|stored| (stored.profile.vendor, stored.profile.model, stored.revision))
            };
            "Redfish, DMI and lower-case vendor names" {
                ("Lenovo", "ThinkSystem SR680a V3")
                    => Ok((RedfishVendor::Lenovo, "ThinkSystem_SR680a_V3".to_string(), "3f2a9c1d7b4e".to_string())),
                ("Dell Inc.", "PowerEdge XE9680")
                    => Ok((RedfishVendor::Dell, "PowerEdge_XE9680".to_string(), "3f2a9c1d7b4e".to_string())),
                ("supermicro", "SYS-821GE-TNHR")
                    => Ok((RedfishVendor::Supermicro, "SYS-821GE-TNHR".to_string(), "3f2a9c1d7b4e".to_string())),
                ("NvidiaGBx00", "GB200 NVL")
                    => Ok((RedfishVendor::NvidiaGBx00, "GB200_NVL".to_string(), "3f2a9c1d7b4e".to_string())),
            }
            "vendors NICo cannot match a host to" {
                ("NVIDIA", "GB200 NVL")
                    => Err("BIOS profile vendor NVIDIA is not a BMC vendor NICo can match a host to".to_string()),
                ("Unknown", "Server")
                    => Err("BIOS profile vendor Unknown is not a BMC vendor NICo can match a host to".to_string()),
                ("Acme", "Server")
                    => Err("BIOS profile vendor Acme is not a BMC vendor NICo can match a host to".to_string()),
            }
        );
    }

    #[tokio::test]
    async fn a_loaded_cache_reads_the_database_again_only_once_invalidated() {
        let stored = stored_bios_profiles(&[profile("Lenovo", "ThinkSystem SR680a V3")]);
        let cache = NamedBiosProfileCache::with_profiles(stored.clone());
        let unreachable = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(50))
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap();

        assert_eq!(
            cache.profiles(&unreachable).await.unwrap().as_ref(),
            &stored
        );
        cache.invalidate();
        assert!(cache.profiles(&unreachable).await.is_err());
    }
}
