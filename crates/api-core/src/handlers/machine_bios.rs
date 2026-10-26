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
use std::sync::Mutex;
use std::time::Duration;

use ::rpc::forge as rpc;
use carbide_machine_controller::handler::named_bios_profile_for_explored_host;
use carbide_utils::redfish::BmcAccessInfo;
use carbide_uuid::machine::MachineId;
use chrono::{TimeDelta, Utc};
use libredfish::Redfish;
use libredfish::model::ODataId;
use model::machine::AnyMachine;
use model::machine_bios::{
    BiosObservationSource, MachineBiosObservation, bios_attribute_registry_location,
    parse_bios_attribute_registry,
};
use serde_json::Value;
use tonic::{Request, Response, Status};

use crate::CarbideError;
use crate::api::{Api, log_request_data};
use crate::handlers::utils::convert_and_log_machine_id;

const REFRESH_REUSE_SECONDS: i64 = 60;
const BMC_READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RegistryCacheKey {
    vendor: String,
    model: String,
    bios_version: String,
    registry: String,
}

impl RegistryCacheKey {
    fn for_machine(machine: &AnyMachine, registry: &str) -> Option<Self> {
        let dmi = machine.status.hardware_info.as_ref()?.dmi_data.as_ref()?;
        if dmi.sys_vendor.is_empty() || dmi.product_name.is_empty() {
            return None;
        }
        Some(Self {
            vendor: dmi.sys_vendor.clone(),
            model: dmi.product_name.clone(),
            bios_version: dmi.bios_version.clone(),
            registry: registry.to_string(),
        })
    }
}

#[derive(Default)]
pub(crate) struct BiosAttributeRegistryCache(
    Mutex<HashMap<RegistryCacheKey, rpc::MachineBiosAttributes>>,
);

impl BiosAttributeRegistryCache {
    fn get(&self, key: &RegistryCacheKey) -> Option<rpc::MachineBiosAttributes> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned()
    }

    fn insert(&self, key: RegistryCacheKey, attributes: rpc::MachineBiosAttributes) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, attributes);
    }
}

pub(crate) async fn get_machine_bios(
    api: &Api,
    request: Request<MachineId>,
) -> Result<Response<rpc::MachineBios>, Status> {
    log_request_data(&request);
    let machine_id: MachineId = convert_and_log_machine_id(Some(request.get_ref()))?;

    let (machine, mut txn) = api.load_machine(&machine_id, Default::default()).await?;
    let profile = bios_profile(api, &mut txn, &machine).await?;
    let observation =
        db::machine_bios_observation::find_by_machine_id(&mut txn, &machine_id).await?;
    txn.commit().await?;

    Ok(Response::new(machine_bios(
        machine_id,
        profile,
        observation,
    )))
}

pub(crate) async fn refresh_machine_bios(
    api: &Api,
    request: Request<MachineId>,
) -> Result<Response<rpc::MachineBios>, Status> {
    log_request_data(&request);
    let machine_id: MachineId = convert_and_log_machine_id(Some(request.get_ref()))?;

    let (machine, mut txn) = api.load_machine(&machine_id, Default::default()).await?;
    let profile = bios_profile(api, &mut txn, &machine).await?;
    let stored = db::machine_bios_observation::find_by_machine_id(&mut txn, &machine_id).await?;
    if let Some(stored) = stored.filter(|observation| {
        observation.is_refresh_within(TimeDelta::seconds(REFRESH_REUSE_SECONDS), Utc::now())
    }) {
        txn.commit().await?;
        return Ok(Response::new(machine_bios(
            machine_id,
            profile,
            Some(stored),
        )));
    }
    let bmc_access_info = bmc_access_info(&mut txn, &machine).await?;
    txn.commit().await?;

    let (bios, pending) = with_bmc_deadline(async {
        let redfish = connect(api, &bmc_access_info, machine_id).await?;
        Ok::<_, CarbideError>((redfish.bios().await?, redfish.pending().await?))
    })
    .await?;
    let bios_version = machine
        .status
        .hardware_info
        .as_ref()
        .and_then(|info| info.dmi_data.as_ref())
        .map(|dmi| dmi.bios_version.clone())
        .filter(|version| !version.is_empty());
    let observation = MachineBiosObservation::from_bios_resource(
        machine_id,
        BiosObservationSource::Refresh,
        Utc::now(),
        bios_version,
        &bios,
        pending,
    )
    .ok_or_else(|| {
        CarbideError::internal(format!(
            "the BMC of machine {machine_id} returned a Bios resource without an Attributes object"
        ))
    })?;

    let mut txn = api.txn_begin().await?;
    db::machine_bios_observation::upsert(&mut txn, &observation).await?;
    txn.commit().await?;

    Ok(Response::new(machine_bios(
        machine_id,
        profile,
        Some(observation),
    )))
}

pub(crate) async fn get_machine_bios_attributes(
    api: &Api,
    request: Request<MachineId>,
) -> Result<Response<rpc::MachineBiosAttributes>, Status> {
    log_request_data(&request);
    let machine_id: MachineId = convert_and_log_machine_id(Some(request.get_ref()))?;

    let (machine, mut txn) = api.load_machine(&machine_id, Default::default()).await?;
    let bmc_access_info = bmc_access_info(&mut txn, &machine).await?;
    txn.commit().await?;

    let attributes = with_bmc_deadline(async {
        let redfish = connect(api, &bmc_access_info, machine_id).await?;
        let bios = redfish.bios().await?;
        let registry = bios
            .get("AttributeRegistry")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CarbideError::internal(format!(
                    "the BMC of machine {machine_id} does not name a BIOS attribute registry"
                ))
            })?;
        let key = RegistryCacheKey::for_machine(&machine, registry);
        if let Some(cached) = key
            .as_ref()
            .and_then(|key| api.bios_attribute_registries.get(key))
        {
            return Ok::<_, CarbideError>(cached);
        }
        let attributes = read_bios_attribute_registry(redfish.as_ref(), registry).await?;
        if let Some(key) = key {
            api.bios_attribute_registries
                .insert(key, attributes.clone());
        }
        Ok::<_, CarbideError>(attributes)
    })
    .await?;

    Ok(Response::new(attributes))
}

async fn bios_profile(
    api: &Api,
    txn: &mut db::Transaction<'_>,
    machine: &AnyMachine,
) -> Result<Option<rpc::MachineBiosProfile>, CarbideError> {
    let Some(bmc_ip) = machine.status.bmc_info.ip else {
        return Ok(None);
    };
    let Some(endpoint) = db::explored_endpoints::find_by_ips(&mut *txn, vec![bmc_ip])
        .await?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    Ok(named_bios_profile_for_explored_host(
        &machine.metadata.labels,
        &api.runtime_config.named_bios_profiles,
        &endpoint.report,
    )
    .map(|(name, profile)| rpc::MachineBiosProfile {
        name: name.clone(),
        attributes_json: Value::Object(profile.attributes.clone().into_iter().collect())
            .to_string(),
    }))
}

async fn bmc_access_info(
    txn: &mut db::Transaction<'_>,
    machine: &AnyMachine,
) -> Result<BmcAccessInfo, CarbideError> {
    let addr = machine.bmc_addr().ok_or_else(|| {
        CarbideError::InvalidArgument("specified machine does not have BMC address".into())
    })?;
    Ok(
        db::machine_interface::lookup_bmc_access_info(&mut *txn, addr.ip(), Some(addr.port()))
            .await?,
    )
}

async fn connect(
    api: &Api,
    bmc_access_info: &BmcAccessInfo,
    machine_id: MachineId,
) -> Result<Box<dyn Redfish>, CarbideError> {
    api.redfish_pool
        .client_by_info(bmc_access_info)
        .await
        .map_err(|inner| CarbideError::RedfishClientCreation {
            inner: Box::new(inner),
            machine_id,
        })
}

async fn with_bmc_deadline<T>(
    read: impl Future<Output = Result<T, CarbideError>>,
) -> Result<T, CarbideError> {
    tokio::time::timeout(BMC_READ_TIMEOUT, read)
        .await
        .map_err(|_| {
            CarbideError::UnavailableError(format!(
                "the BMC did not answer within {} seconds",
                BMC_READ_TIMEOUT.as_secs()
            ))
        })?
}

async fn read_bios_attribute_registry(
    redfish: &dyn Redfish,
    registry: &str,
) -> Result<rpc::MachineBiosAttributes, CarbideError> {
    let file = redfish
        .get_resource(ODataId::from(format!("/redfish/v1/Registries/{registry}")))
        .await?;
    let location = bios_attribute_registry_location(file.raw.get()).map_err(|error| {
        CarbideError::internal(format!(
            "BIOS attribute registry {registry} has no readable location: {error}"
        ))
    })?;
    let document = redfish.get_resource(ODataId::from(location)).await?;
    let attributes = parse_bios_attribute_registry(document.raw.get()).map_err(|error| {
        CarbideError::internal(format!(
            "BIOS attribute registry {registry} could not be parsed: {error}"
        ))
    })?;
    Ok(rpc::MachineBiosAttributes {
        registry: registry.to_string(),
        attributes: attributes.into_iter().map(Into::into).collect(),
    })
}

fn machine_bios(
    machine_id: MachineId,
    profile: Option<rpc::MachineBiosProfile>,
    observation: Option<MachineBiosObservation>,
) -> rpc::MachineBios {
    rpc::MachineBios {
        machine_id: Some(machine_id),
        profile,
        observation: observation.map(Into::into),
    }
}
