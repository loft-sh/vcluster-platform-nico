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

use model::machine_bios::{BiosAttributeDefinition, MachineBiosObservation};

use crate as rpc;

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
        });

        assert_eq!(observation.observed_by, "post_reboot");
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
}
