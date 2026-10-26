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

use carbide_uuid::machine::MachineId;
use chrono::{DateTime, TimeDelta, Utc};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, strum_macros::EnumString, strum_macros::IntoStaticStr,
)]
#[strum(serialize_all = "snake_case")]
pub enum BiosObservationSource {
    LifecycleGate,
    PostReboot,
    Refresh,
}

impl BiosObservationSource {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MachineBiosObservation {
    pub machine_id: MachineId,
    pub observed_at: DateTime<Utc>,
    pub observed_by: BiosObservationSource,
    pub bios_version: Option<String>,
    pub attribute_registry: Option<String>,
    pub settings_applied_at: Option<DateTime<Utc>>,
    pub current: Map<String, Value>,
    pub pending: Map<String, Value>,
    pub applied_revision: Option<String>,
}

impl MachineBiosObservation {
    pub fn from_bios_resource(
        machine_id: MachineId,
        observed_by: BiosObservationSource,
        observed_at: DateTime<Utc>,
        bios_version: Option<String>,
        bios: &HashMap<String, Value>,
        pending: HashMap<String, Value>,
    ) -> Option<Self> {
        let current = bios.get("Attributes")?.as_object()?.clone();
        Some(Self {
            machine_id,
            observed_at,
            observed_by,
            bios_version,
            attribute_registry: bios
                .get("AttributeRegistry")
                .and_then(Value::as_str)
                .map(str::to_string),
            settings_applied_at: bios
                .get("@Redfish.Settings")
                .and_then(|settings| settings.get("Time"))
                .and_then(Value::as_str)
                .and_then(|time| DateTime::parse_from_rfc3339(time).ok())
                .map(|time| time.with_timezone(&Utc)),
            current,
            pending: pending.into_iter().collect(),
            applied_revision: None,
        })
    }

    pub fn is_refresh_within(&self, window: TimeDelta, now: DateTime<Utc>) -> bool {
        self.observed_by == BiosObservationSource::Refresh
            && now.signed_duration_since(self.observed_at) < window
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BiosProfile {
    pub name: String,
    pub vendor: String,
    pub model: String,
    pub attribute_registry: Option<String>,
    pub attributes: Map<String, Value>,
    pub revision: String,
    pub updated_at: Option<DateTime<Utc>>,
}

impl BiosProfile {
    pub fn content_revision(&self) -> String {
        let mut hash = Sha256::new();
        let mut write = |value: &str| {
            hash.update(value.as_bytes());
            hash.update([0]);
        };
        write(&self.vendor);
        write(&self.model);
        write(self.attribute_registry.as_deref().unwrap_or_default());
        let mut attributes: Vec<_> = self.attributes.iter().collect();
        attributes.sort_by_key(|(name, _)| *name);
        for (name, value) in attributes {
            write(name);
            write(&value.to_string());
        }
        hex::encode(hash.finalize())[..12].to_string()
    }
}

pub fn parse_bios_profile_attributes(attributes_json: &str) -> Result<Map<String, Value>, String> {
    let attributes = match serde_json::from_str::<Value>(attributes_json) {
        Ok(Value::Object(attributes)) => attributes,
        Ok(_) => return Err("BIOS profile attributes must be a JSON object".to_string()),
        Err(error) => {
            return Err(format!(
                "BIOS profile attributes are not valid JSON: {error}"
            ));
        }
    };
    if let Some(name) = attributes.iter().find_map(|(name, value)| {
        (!matches!(value, Value::String(_) | Value::Number(_) | Value::Bool(_))).then_some(name)
    }) {
        return Err(format!(
            "BIOS profile attribute {name} must be a string, number or boolean"
        ));
    }
    Ok(attributes)
}

#[derive(Clone, Debug, PartialEq)]
pub struct BiosAttributeDefinition {
    pub name: String,
    pub display_name: String,
    pub attribute_type: String,
    pub read_only: bool,
    pub allowed_values: Vec<String>,
    pub lower_bound: Option<f64>,
    pub upper_bound: Option<f64>,
    pub help_text: Option<String>,
    pub default_value: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RegistryFile {
    #[serde(default)]
    location: Vec<RegistryLocation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RegistryLocation {
    uri: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AttributeRegistry {
    registry_entries: RegistryEntries,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RegistryEntries {
    #[serde(default)]
    attributes: Vec<RegistryAttribute>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RegistryAttribute {
    attribute_name: String,
    display_name: Option<String>,
    #[serde(rename = "Type")]
    attribute_type: Option<String>,
    read_only: Option<bool>,
    value: Option<Vec<RegistryValue>>,
    lower_bound: Option<f64>,
    upper_bound: Option<f64>,
    help_text: Option<String>,
    default_value: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RegistryValue {
    value_name: String,
}

impl From<RegistryAttribute> for BiosAttributeDefinition {
    fn from(attribute: RegistryAttribute) -> Self {
        Self {
            name: attribute.attribute_name,
            display_name: attribute.display_name.unwrap_or_default(),
            attribute_type: attribute.attribute_type.unwrap_or_default(),
            read_only: attribute.read_only.unwrap_or_default(),
            allowed_values: attribute
                .value
                .unwrap_or_default()
                .into_iter()
                .map(|value| value.value_name)
                .collect(),
            lower_bound: attribute.lower_bound,
            upper_bound: attribute.upper_bound,
            help_text: attribute.help_text,
            default_value: match attribute.default_value {
                None | Some(Value::Null) => None,
                Some(Value::String(text)) => Some(text),
                Some(other) => Some(other.to_string()),
            },
        }
    }
}

pub fn bios_attribute_registry_location(registry_file: &str) -> Result<String, serde_json::Error> {
    let file: RegistryFile = serde_json::from_str(registry_file)?;
    file.location
        .into_iter()
        .find_map(|location| location.uri)
        .ok_or_else(|| serde::de::Error::missing_field("Location[].Uri"))
}

pub fn parse_bios_attribute_registry(
    registry: &str,
) -> Result<Vec<BiosAttributeDefinition>, serde_json::Error> {
    let registry: AttributeRegistry = serde_json::from_str(registry)?;
    Ok(registry
        .registry_entries
        .attributes
        .into_iter()
        .map(BiosAttributeDefinition::from)
        .collect())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use carbide_test_support::value_scenarios;
    use serde_json::json;

    use super::*;

    const LENOVO_BIOS: &str = include_str!("machine_bios/test_data/lenovo_sr680a_v3_bios.json");
    const LENOVO_REGISTRY: &str =
        include_str!("machine_bios/test_data/lenovo_sr680a_v3_bios_registry.json");

    fn machine_id() -> MachineId {
        MachineId::from_str("fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg").unwrap()
    }

    fn at(second: u32) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-25T23:52:15Z")
            .unwrap()
            .with_timezone(&Utc)
            + TimeDelta::seconds(second.into())
    }

    #[test]
    fn observation_keeps_every_bmc_attribute_and_the_settings_metadata() {
        let bios: HashMap<String, Value> = serde_json::from_str(LENOVO_BIOS).unwrap();
        let pending = HashMap::from([(
            "DevicesandIOPorts_IntelVTforDirectedIOVTd".to_string(),
            json!("Enabled"),
        )]);

        let observation = MachineBiosObservation::from_bios_resource(
            machine_id(),
            BiosObservationSource::LifecycleGate,
            at(0),
            Some("R3X318M".to_string()),
            &bios,
            pending,
        )
        .expect("the Bios resource has Attributes");

        assert_eq!(
            observation,
            MachineBiosObservation {
                machine_id: machine_id(),
                observed_at: at(0),
                observed_by: BiosObservationSource::LifecycleGate,
                bios_version: Some("R3X318M".to_string()),
                attribute_registry: Some("BiosAttributeRegistry.1.0.0".to_string()),
                settings_applied_at: Some(at(0)),
                current: json!({
                    "OperatingModes_ChooseOperatingMode": "CustomMode",
                    "DevicesandIOPorts_IntelVTforDirectedIOVTd": "Disabled",
                    "SystemRecovery_POSTWatchdogTimer": "Disabled",
                    "SystemRecovery_POSTWatchdogTimerValue": 5,
                    "TrustedComputingGroup_DeviceStatus": "TPM2.0 Device present."
                })
                .as_object()
                .unwrap()
                .clone(),
                pending: json!({"DevicesandIOPorts_IntelVTforDirectedIOVTd": "Enabled"})
                    .as_object()
                    .unwrap()
                    .clone(),
                applied_revision: None,
            }
        );
    }

    #[test]
    fn observation_needs_an_attributes_object_and_tolerates_missing_metadata() {
        value_scenarios!(
            run = |bios: Value| {
                let bios: HashMap<String, Value> = serde_json::from_value(bios).unwrap();
                MachineBiosObservation::from_bios_resource(
                    machine_id(),
                    BiosObservationSource::Refresh,
                    at(0),
                    None,
                    &bios,
                    HashMap::new(),
                )
                .map(|observation| (observation.attribute_registry, observation.settings_applied_at))
            };
            "no Attributes object is not an observation" {
                json!({"AttributeRegistry": "R"}) => None,
                json!({"Attributes": "Enabled"}) => None,
            }
            "registry and settings time are optional" {
                json!({"Attributes": {}}) => Some((None, None)),
                json!({"Attributes": {}, "@Redfish.Settings": {"Time": "not a time"}}) => Some((None, None)),
            }
        );
    }

    #[test]
    fn observation_sources_round_trip_through_their_stored_names() {
        value_scenarios!(
            run = |source: BiosObservationSource| {
                let stored = source.as_str();
                (stored, BiosObservationSource::from_str(stored).ok())
            };
            "stored names" {
                BiosObservationSource::LifecycleGate => ("lifecycle_gate", Some(BiosObservationSource::LifecycleGate)),
                BiosObservationSource::PostReboot => ("post_reboot", Some(BiosObservationSource::PostReboot)),
                BiosObservationSource::Refresh => ("refresh", Some(BiosObservationSource::Refresh)),
            }
        );
    }

    #[test]
    fn only_a_recent_refresh_is_reused() {
        let observation = |observed_by, observed_at| MachineBiosObservation {
            machine_id: machine_id(),
            observed_at,
            observed_by,
            bios_version: None,
            attribute_registry: None,
            settings_applied_at: None,
            current: Map::new(),
            pending: Map::new(),
            applied_revision: None,
        };
        value_scenarios!(
            run = |(observed_by, observed_at, now): (BiosObservationSource, DateTime<Utc>, DateTime<Utc>)| {
                observation(observed_by, observed_at).is_refresh_within(TimeDelta::seconds(60), now)
            };
            "a refresh inside the window is reused" {
                (BiosObservationSource::Refresh, at(0), at(59)) => true,
            }
            "a refresh at or past the window reads the BMC again" {
                (BiosObservationSource::Refresh, at(0), at(60)) => false,
            }
            "a lifecycle observation never suppresses a refresh" {
                (BiosObservationSource::LifecycleGate, at(0), at(1)) => false,
                (BiosObservationSource::PostReboot, at(0), at(1)) => false,
            }
        );
    }

    #[test]
    fn bios_profile_attributes_are_an_object_of_scalars() {
        value_scenarios!(
            run = |attributes_json: &str| parse_bios_profile_attributes(attributes_json).map(Value::Object);
            "string, number and boolean values are kept" {
                r#"{"A": "Enabled", "B": 5, "C": true}"# => Ok(json!({"A": "Enabled", "B": 5, "C": true})),
                "{}" => Ok(json!({})),
            }
            "anything but an object of scalars is refused" {
                r#"["A"]"# => Err("BIOS profile attributes must be a JSON object".to_string()),
                r#"{"A": null}"# => Err("BIOS profile attribute A must be a string, number or boolean".to_string()),
                r#"{"A": ["Enabled"]}"# => Err("BIOS profile attribute A must be a string, number or boolean".to_string()),
                r#"{"A": {"B": 1}}"# => Err("BIOS profile attribute A must be a string, number or boolean".to_string()),
            }
        );
        assert!(
            parse_bios_profile_attributes("Enabled")
                .unwrap_err()
                .starts_with("BIOS profile attributes are not valid JSON")
        );
    }

    #[test]
    fn bios_profile_content_revision_follows_the_content_only() {
        let profile = |attributes: Value, revision: &str| BiosProfile {
            name: "lenovo-h200".to_string(),
            vendor: "Lenovo".to_string(),
            model: "ThinkSystem SR680a V3".to_string(),
            attribute_registry: Some("BiosAttributeRegistry.1.0.0".to_string()),
            attributes: attributes.as_object().unwrap().clone(),
            revision: revision.to_string(),
            updated_at: None,
        };
        let base = profile(json!({"A": "Enabled", "B": 5}), "");

        assert_eq!(base.content_revision().len(), 12);
        assert_eq!(
            base.content_revision(),
            profile(json!({"B": 5, "A": "Enabled"}), "r2").content_revision()
        );
        assert_ne!(
            base.content_revision(),
            profile(json!({"A": "Disabled", "B": 5}), "").content_revision()
        );
    }

    #[test]
    fn registry_attributes_carry_their_constraints() {
        type Constraints<'a> = (
            &'a str,
            &'a str,
            bool,
            Vec<String>,
            Option<f64>,
            Option<f64>,
            Option<&'a str>,
        );
        let attributes = parse_bios_attribute_registry(LENOVO_REGISTRY).unwrap();
        let strings = |values: &[&str]| -> Vec<String> {
            values.iter().map(|value| value.to_string()).collect()
        };

        assert_eq!(
            attributes
                .iter()
                .map(|attribute| (
                    attribute.name.as_str(),
                    attribute.attribute_type.as_str(),
                    attribute.read_only,
                    attribute.allowed_values.clone(),
                    attribute.lower_bound,
                    attribute.upper_bound,
                    attribute.default_value.as_deref(),
                ))
                .collect::<Vec<Constraints>>(),
            vec![
                (
                    "OperatingModes_ChooseOperatingMode",
                    "Enumeration",
                    false,
                    strings(&[
                        "MinimalPower",
                        "Efficiency_FavorPower",
                        "Efficiency_FavorPerformance",
                        "CustomMode",
                        "MaximumPerformance",
                    ]),
                    None,
                    None,
                    Some("Efficiency_FavorPerformance"),
                ),
                (
                    "DevicesandIOPorts_IntelVTforDirectedIOVTd",
                    "Enumeration",
                    false,
                    strings(&["Disabled", "Enabled"]),
                    None,
                    None,
                    Some("Enabled"),
                ),
                (
                    "SystemRecovery_POSTWatchdogTimer",
                    "Enumeration",
                    false,
                    strings(&["Disabled", "Enabled"]),
                    None,
                    None,
                    Some("Disabled"),
                ),
                (
                    "SystemRecovery_POSTWatchdogTimerValue",
                    "Integer",
                    false,
                    vec![],
                    Some(5.0),
                    Some(20.0),
                    Some("5"),
                ),
                (
                    "TrustedComputingGroup_DeviceStatus",
                    "String",
                    true,
                    vec![],
                    None,
                    None,
                    None,
                ),
            ]
        );
        assert_eq!(
            attributes[3],
            BiosAttributeDefinition {
                name: "SystemRecovery_POSTWatchdogTimerValue".to_string(),
                display_name: "POST Watchdog Timer Value".to_string(),
                attribute_type: "Integer".to_string(),
                read_only: false,
                allowed_values: vec![],
                lower_bound: Some(5.0),
                upper_bound: Some(20.0),
                help_text: Some(
                    "Enter POST loader Watchdog timer value in minutes from the specified range(5-20)."
                        .to_string()
                ),
                default_value: Some("5".to_string()),
            }
        );
    }

    #[test]
    fn registry_values_from_other_vendors() {
        value_scenarios!(
            run = |attribute: Value| {
                let attribute: RegistryAttribute = serde_json::from_value(attribute).unwrap();
                let attribute = BiosAttributeDefinition::from(attribute);
                (attribute.read_only, attribute.allowed_values, attribute.default_value)
            };
            "numeric and boolean defaults render as text" {
                json!({"AttributeName": "A", "DefaultValue": 3}) => (false, vec![], Some("3".to_string())),
                json!({"AttributeName": "A", "DefaultValue": true}) => (false, vec![], Some("true".to_string())),
            }
            "null collections and flags are empty" {
                json!({"AttributeName": "A", "ReadOnly": null, "Value": null, "DefaultValue": null}) => (false, vec![], None),
            }
        );
    }

    #[test]
    fn registry_file_points_at_the_registry_document() {
        value_scenarios!(
            run = |file: Value| bios_attribute_registry_location(&file.to_string()).ok();
            "the first location with a URI" {
                json!({"Location": [{"Language": "en", "Uri": "/redfish/v1/schemas/registries/BiosAttributeRegistry.1.0.0.json"}]})
                    => Some("/redfish/v1/schemas/registries/BiosAttributeRegistry.1.0.0.json".to_string()),
                json!({"Location": [{"Language": "en", "PublicationUri": "https://example.com/r.json"}, {"Uri": "/redfish/v1/r.json"}]})
                    => Some("/redfish/v1/r.json".to_string()),
            }
            "no local copy" {
                json!({"Location": []}) => None,
                json!({}) => None,
            }
        );
    }
}
