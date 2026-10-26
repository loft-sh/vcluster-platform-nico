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

use model::machine_bios::{
    BiosAttributeDefinition, BiosProfile, MachineBiosObservation, parse_bios_profile_attributes,
};

use crate as rpc;
use crate::errors::RpcDataConversionError;

impl From<MachineBiosObservation> for rpc::forge::MachineBiosObservation {
    fn from(observation: MachineBiosObservation) -> Self {
        rpc::forge::MachineBiosObservation {
            observed_at: Some(observation.observed_at.into()),
            observed_by: observation.observed_by.as_str().to_string(),
            bios_version: observation.bios_version,
            attribute_registry: observation.attribute_registry,
            settings_applied_at: observation.settings_applied_at.map(Into::into),
            current_json: serde_json::Value::Object(observation.current).to_string(),
            pending_json: serde_json::Value::Object(observation.pending).to_string(),
            applied_revision: observation.applied_revision,
        }
    }
}

impl TryFrom<rpc::forge::BiosProfile> for BiosProfile {
    type Error = RpcDataConversionError;

    fn try_from(profile: rpc::forge::BiosProfile) -> Result<Self, Self::Error> {
        for (field, value) in [
            ("name", &profile.name),
            ("vendor", &profile.vendor),
            ("model", &profile.model),
        ] {
            if value.trim().is_empty() {
                return Err(RpcDataConversionError::MissingArgument(field));
            }
        }
        let attributes = parse_bios_profile_attributes(&profile.attributes_json)
            .map_err(RpcDataConversionError::InvalidArgument)?;
        let mut bios_profile = BiosProfile {
            name: profile.name,
            vendor: profile.vendor,
            model: profile.model,
            attribute_registry: profile
                .attribute_registry
                .filter(|registry| !registry.trim().is_empty()),
            attributes,
            revision: profile.revision,
            updated_at: None,
        };
        if bios_profile.revision.trim().is_empty() {
            bios_profile.revision = bios_profile.content_revision();
        }
        Ok(bios_profile)
    }
}

impl From<BiosProfile> for rpc::forge::BiosProfile {
    fn from(profile: BiosProfile) -> Self {
        rpc::forge::BiosProfile {
            name: profile.name,
            vendor: profile.vendor,
            model: profile.model,
            attribute_registry: profile.attribute_registry,
            attributes_json: serde_json::Value::Object(profile.attributes).to_string(),
            revision: profile.revision,
            updated_at: profile.updated_at.map(Into::into),
        }
    }
}

impl From<BiosAttributeDefinition> for rpc::forge::MachineBiosAttribute {
    fn from(attribute: BiosAttributeDefinition) -> Self {
        rpc::forge::MachineBiosAttribute {
            name: attribute.name,
            display_name: attribute.display_name,
            r#type: attribute.attribute_type,
            read_only: attribute.read_only,
            allowed_values: attribute.allowed_values,
            lower_bound: attribute.lower_bound,
            upper_bound: attribute.upper_bound,
            help_text: attribute.help_text,
            default_value: attribute.default_value,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use carbide_test_support::value_scenarios;
    use carbide_uuid::machine::MachineId;
    use chrono::{DateTime, Utc};
    use model::machine_bios::BiosObservationSource;
    use serde_json::json;

    use super::*;

    #[test]
    fn observation_values_survive_the_json_fields() {
        let observed_at = DateTime::parse_from_rfc3339("2026-10-25T23:52:15Z")
            .unwrap()
            .with_timezone(&Utc);
        let current = json!({
            "OperatingModes_ChooseOperatingMode": "CustomMode",
            "SystemRecovery_POSTWatchdogTimerValue": 5,
            "Processors_HyperThreading": true
        });
        let pending = json!({"OperatingModes_ChooseOperatingMode": "MaximumPerformance"});

        let observation = rpc::forge::MachineBiosObservation::from(MachineBiosObservation {
            machine_id: MachineId::from_str(
                "fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg",
            )
            .unwrap(),
            observed_at,
            observed_by: BiosObservationSource::PostReboot,
            bios_version: Some("R3X318M".to_string()),
            attribute_registry: None,
            settings_applied_at: None,
            current: current.as_object().unwrap().clone(),
            pending: pending.as_object().unwrap().clone(),
            applied_revision: Some("3f2a9c1d7b4e".to_string()),
        });

        assert_eq!(observation.observed_by, "post_reboot");
        assert_eq!(
            observation.applied_revision.as_deref(),
            Some("3f2a9c1d7b4e")
        );
        assert_eq!(
            observation
                .observed_at
                .map(DateTime::<Utc>::try_from)
                .transpose()
                .unwrap(),
            Some(observed_at)
        );
        assert_eq!(observation.settings_applied_at, None);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&observation.current_json).unwrap(),
            current
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&observation.pending_json).unwrap(),
            pending
        );
    }

    fn rpc_profile(
        name: &str,
        vendor: &str,
        attributes_json: &str,
        revision: &str,
    ) -> rpc::forge::BiosProfile {
        rpc::forge::BiosProfile {
            name: name.to_string(),
            vendor: vendor.to_string(),
            model: "ThinkSystem SR680a V3".to_string(),
            attribute_registry: Some("BiosAttributeRegistry.1.0.0".to_string()),
            attributes_json: attributes_json.to_string(),
            revision: revision.to_string(),
            updated_at: None,
        }
    }

    #[test]
    fn bios_profile_upserts_are_validated() {
        let valid = r#"{"OperatingModes_ChooseOperatingMode": "MaximumPerformance", "SystemRecovery_POSTWatchdogTimerValue": 5}"#;
        value_scenarios!(
            run = |profile: rpc::forge::BiosProfile| {
                BiosProfile::try_from(profile)
                    .map(|profile| (profile.name, profile.revision))
                    .map_err(|error| error.to_string())
            };
            "a complete profile keeps the platform revision" {
                rpc_profile("lenovo-h200", "Lenovo", valid, "3f2a9c1d7b4e")
                    => Ok(("lenovo-h200".to_string(), "3f2a9c1d7b4e".to_string())),
            }
            "name, vendor and model are required" {
                rpc_profile("", "Lenovo", valid, "r1") => Err("argument name is missing".to_string()),
                rpc_profile("lenovo-h200", " ", valid, "r1") => Err("argument vendor is missing".to_string()),
            }
            "attributes must be a JSON object of scalars" {
                rpc_profile("lenovo-h200", "Lenovo", "[]", "r1")
                    => Err("argument is invalid: BIOS profile attributes must be a JSON object".to_string()),
                rpc_profile("lenovo-h200", "Lenovo", r#"{"A": {"B": 1}}"#, "r1")
                    => Err("argument is invalid: BIOS profile attribute A must be a string, number or boolean".to_string()),
            }
        );
        let mut no_model = rpc_profile("lenovo-h200", "Lenovo", valid, "r1");
        no_model.model = String::new();
        assert_eq!(
            BiosProfile::try_from(no_model).unwrap_err().to_string(),
            "argument model is missing"
        );
    }

    #[test]
    fn bios_profile_without_a_revision_gets_one_from_its_content() {
        let attributes = r#"{"OperatingModes_ChooseOperatingMode": "MaximumPerformance"}"#;
        let mut no_registry = rpc_profile("lenovo-h200", "Lenovo", attributes, "");
        no_registry.attribute_registry = Some(String::new());
        let profile = BiosProfile::try_from(no_registry).unwrap();

        assert_eq!(profile.attribute_registry, None);
        assert_eq!(profile.revision, profile.content_revision());
        assert_eq!(
            rpc::forge::BiosProfile::from(profile.clone()),
            rpc::forge::BiosProfile {
                name: "lenovo-h200".to_string(),
                vendor: "Lenovo".to_string(),
                model: "ThinkSystem SR680a V3".to_string(),
                attribute_registry: None,
                attributes_json: attributes.replace(": ", ":"),
                revision: profile.revision,
                updated_at: None,
            }
        );
    }
}
