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

use std::collections::BTreeSet;
use std::fmt::Write as _;

use ::rpc::admin_cli::OutputFormat;
use ::rpc::forge as forgerpc;
use carbide_uuid::machine::MachineId;
use prettytable::{Cell, Row, Table};
use serde::Serialize;
use serde_json::{Map, Value};

use super::args::BiosArgs;
use crate::errors::CarbideCliResult;
use crate::rpc::ApiClient;

const SUMMARY_LABEL_WIDTH: usize = 21;

#[derive(Debug, Serialize)]
struct BiosReport {
    machine_id: Option<MachineId>,
    profile: Option<ProfileReport>,
    observation: Option<ObservationReport>,
}

#[derive(Debug, Serialize)]
struct ProfileReport {
    name: String,
    revision: Option<String>,
    attributes: Map<String, Value>,
}

#[derive(Debug, Serialize)]
struct ObservationReport {
    observed_at: Option<String>,
    observed_by: String,
    bios_version: Option<String>,
    attribute_registry: Option<String>,
    settings_applied_at: Option<String>,
    current: Map<String, Value>,
    pending: Map<String, Value>,
    applied_revision: Option<String>,
}

impl TryFrom<forgerpc::MachineBios> for BiosReport {
    type Error = serde_json::Error;

    fn try_from(bios: forgerpc::MachineBios) -> Result<Self, Self::Error> {
        Ok(Self {
            machine_id: bios.machine_id,
            profile: bios
                .profile
                .map(|profile| {
                    Ok::<_, serde_json::Error>(ProfileReport {
                        name: profile.name,
                        revision: profile.revision,
                        attributes: serde_json::from_str(&profile.attributes_json)?,
                    })
                })
                .transpose()?,
            observation: bios
                .observation
                .map(|observation| {
                    Ok::<_, serde_json::Error>(ObservationReport {
                        observed_at: observation.observed_at.map(|time| time.to_string()),
                        observed_by: observation.observed_by,
                        bios_version: observation.bios_version,
                        attribute_registry: observation.attribute_registry,
                        settings_applied_at: observation
                            .settings_applied_at
                            .map(|time| time.to_string()),
                        current: serde_json::from_str(&observation.current_json)?,
                        pending: serde_json::from_str(&observation.pending_json)?,
                        applied_revision: observation.applied_revision,
                    })
                })
                .transpose()?,
        })
    }
}

#[derive(Debug, Serialize)]
struct AttributesReport {
    registry: String,
    attributes: Vec<AttributeRow>,
}

#[derive(Debug, Serialize)]
struct AttributeRow {
    name: String,
    display_name: String,
    #[serde(rename = "type")]
    attribute_type: String,
    read_only: bool,
    allowed_values: Vec<String>,
    lower_bound: Option<f64>,
    upper_bound: Option<f64>,
    help_text: Option<String>,
    default_value: Option<String>,
}

impl From<forgerpc::MachineBiosAttributes> for AttributesReport {
    fn from(attributes: forgerpc::MachineBiosAttributes) -> Self {
        Self {
            registry: attributes.registry,
            attributes: attributes
                .attributes
                .into_iter()
                .map(|attribute| AttributeRow {
                    name: attribute.name,
                    display_name: attribute.display_name,
                    attribute_type: attribute.r#type,
                    read_only: attribute.read_only,
                    allowed_values: attribute.allowed_values,
                    lower_bound: attribute.lower_bound,
                    upper_bound: attribute.upper_bound,
                    help_text: attribute.help_text,
                    default_value: attribute.default_value,
                })
                .collect(),
        }
    }
}

pub(super) async fn handle_show(
    args: BiosArgs,
    output_format: OutputFormat,
    api_client: &ApiClient,
) -> CarbideCliResult<()> {
    let bios = api_client.get_machine_bios(args.machine_id).await?;
    print_bios(BiosReport::try_from(bios)?, output_format)
}

pub(super) async fn handle_refresh(
    args: BiosArgs,
    output_format: OutputFormat,
    api_client: &ApiClient,
) -> CarbideCliResult<()> {
    let bios = api_client.refresh_machine_bios(args.machine_id).await?;
    print_bios(BiosReport::try_from(bios)?, output_format)
}

pub(super) async fn handle_apply(args: BiosArgs, api_client: &ApiClient) -> CarbideCliResult<()> {
    let response = api_client.apply_machine_bios(args.machine_id).await?;
    if response.accepted {
        println!(
            "Applying the bios-profile of machine {}; NICo re-checks its BIOS and boot configuration now",
            args.machine_id
        );
    } else {
        println!(
            "NICo did not accept applying the bios-profile of machine {}",
            args.machine_id
        );
    }
    Ok(())
}

pub(super) async fn handle_attributes(
    args: BiosArgs,
    output_format: OutputFormat,
    api_client: &ApiClient,
) -> CarbideCliResult<()> {
    let attributes = api_client
        .get_machine_bios_attributes(args.machine_id)
        .await?;
    let report = AttributesReport::from(attributes);
    match output_format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        OutputFormat::Yaml => println!("{}", serde_yaml::to_string(&report)?),
        OutputFormat::AsciiTable | OutputFormat::Csv => print!("{}", render_attributes(&report)),
    }
    Ok(())
}

fn print_bios(report: BiosReport, output_format: OutputFormat) -> CarbideCliResult<()> {
    match output_format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        OutputFormat::Yaml => println!("{}", serde_yaml::to_string(&report)?),
        OutputFormat::AsciiTable | OutputFormat::Csv => print!("{}", render_bios(&report)),
    }
    Ok(())
}

fn display_value(value: Option<&Value>) -> String {
    match value {
        None => "-".to_string(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

fn render_bios(report: &BiosReport) -> String {
    let mut out = String::new();
    let dash = |value: Option<&String>| value.map_or("-", String::as_str).to_string();
    let width = SUMMARY_LABEL_WIDTH;
    let machine_id = report
        .machine_id
        .map(|id| id.to_string())
        .unwrap_or_default();
    let observation = report.observation.as_ref();
    let profile = report.profile.as_ref();

    let _ = writeln!(out, "BIOS for machine {machine_id}");
    let _ = writeln!(
        out,
        "{:<width$} {}",
        "Profile:",
        dash(profile.map(|profile| &profile.name))
    );
    let _ = writeln!(
        out,
        "{:<width$} {}",
        "Profile revision:",
        dash(profile.and_then(|profile| profile.revision.as_ref()))
    );
    let _ = writeln!(
        out,
        "{:<width$} {}",
        "Applied revision:",
        dash(observation.and_then(|observation| observation.applied_revision.as_ref()))
    );
    let _ = writeln!(
        out,
        "{:<width$} {}",
        "Observed at:",
        observation.map_or_else(
            || "-".to_string(),
            |observation| format!(
                "{} ({})",
                dash(observation.observed_at.as_ref()),
                observation.observed_by
            )
        )
    );
    let _ = writeln!(
        out,
        "{:<width$} {}",
        "BIOS version:",
        dash(observation.and_then(|observation| observation.bios_version.as_ref()))
    );
    let _ = writeln!(
        out,
        "{:<width$} {}",
        "Attribute registry:",
        dash(observation.and_then(|observation| observation.attribute_registry.as_ref()))
    );
    let _ = writeln!(
        out,
        "{:<width$} {}",
        "Settings applied at:",
        dash(observation.and_then(|observation| observation.settings_applied_at.as_ref()))
    );

    let names: BTreeSet<&String> = observation
        .into_iter()
        .flat_map(|observation| observation.current.keys().chain(observation.pending.keys()))
        .chain(
            profile
                .into_iter()
                .flat_map(|profile| profile.attributes.keys()),
        )
        .collect();
    let mut table = Table::new();
    table.set_titles(Row::new(
        ["Attribute", "Current", "Pending", "Profile"]
            .into_iter()
            .map(Cell::new)
            .collect(),
    ));
    if names.is_empty() {
        table.add_row(Row::new(vec![Cell::new("(none)")]));
    }
    for name in names {
        table.add_row(Row::new(vec![
            Cell::new(name),
            Cell::new(&display_value(
                observation.and_then(|observation| observation.current.get(name)),
            )),
            Cell::new(&display_value(
                observation.and_then(|observation| observation.pending.get(name)),
            )),
            Cell::new(&display_value(
                profile.and_then(|profile| profile.attributes.get(name)),
            )),
        ]));
    }
    let _ = write!(out, "\n{table}");
    out
}

fn render_attributes(report: &AttributesReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "BIOS attribute registry {}", report.registry);
    let mut table = Table::new();
    table.set_titles(Row::new(
        [
            "Attribute",
            "Display Name",
            "Type",
            "Read Only",
            "Allowed Values",
            "Range",
            "Default",
        ]
        .into_iter()
        .map(Cell::new)
        .collect(),
    ));
    if report.attributes.is_empty() {
        table.add_row(Row::new(vec![Cell::new("(none)")]));
    }
    for attribute in &report.attributes {
        let range = match (attribute.lower_bound, attribute.upper_bound) {
            (None, None) => "-".to_string(),
            (lower, upper) => format!(
                "{}..{}",
                lower.map(|bound| bound.to_string()).unwrap_or_default(),
                upper.map(|bound| bound.to_string()).unwrap_or_default()
            ),
        };
        table.add_row(Row::new(vec![
            Cell::new(&attribute.name),
            Cell::new(&attribute.display_name),
            Cell::new(&attribute.attribute_type),
            Cell::new(&attribute.read_only.to_string()),
            Cell::new(&if attribute.allowed_values.is_empty() {
                "-".to_string()
            } else {
                attribute.allowed_values.join(", ")
            }),
            Cell::new(&range),
            Cell::new(attribute.default_value.as_deref().unwrap_or("-")),
        ]));
    }
    let _ = write!(out, "{table}");
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn bios_table_lines_up_current_pending_and_profile_values() {
        let table = render_bios(&BiosReport {
            machine_id: None,
            profile: Some(ProfileReport {
                name: "lenovo-h200".to_string(),
                revision: Some("3f2a9c1d7b4e".to_string()),
                attributes: object(json!({
                    "OperatingModes_ChooseOperatingMode": "MaximumPerformance",
                    "Processors_HyperThreading": "Disabled"
                })),
            }),
            observation: Some(ObservationReport {
                observed_at: Some("2026-10-25T23:52:15Z".to_string()),
                observed_by: "refresh".to_string(),
                bios_version: Some("R3X318M".to_string()),
                attribute_registry: Some("BiosAttributeRegistry.1.0.0".to_string()),
                settings_applied_at: None,
                current: object(json!({
                    "OperatingModes_ChooseOperatingMode": "CustomMode",
                    "SystemRecovery_POSTWatchdogTimerValue": 5
                })),
                pending: object(json!({
                    "OperatingModes_ChooseOperatingMode": "MaximumPerformance"
                })),
                applied_revision: Some("9b8a7c6d5e4f".to_string()),
            }),
        });

        for expected in [
            "Profile:              lenovo-h200",
            "Profile revision:     3f2a9c1d7b4e",
            "Applied revision:     9b8a7c6d5e4f",
            "Observed at:          2026-10-25T23:52:15Z (refresh)",
            "BIOS version:         R3X318M",
            "Attribute registry:   BiosAttributeRegistry.1.0.0",
            "Settings applied at:  -",
            "| Attribute ",
            "| Current ",
            "| Pending ",
            "| Profile ",
            "| OperatingModes_ChooseOperatingMode    | CustomMode | MaximumPerformance | MaximumPerformance |",
            "| Processors_HyperThreading             | -          | -                  | Disabled           |",
            "| SystemRecovery_POSTWatchdogTimerValue | 5          | -                  | -                  |",
        ] {
            assert!(table.contains(expected), "missing {expected:?} in\n{table}");
        }
    }

    #[test]
    fn bios_table_without_observation_or_profile() {
        let table = render_bios(&BiosReport {
            machine_id: None,
            profile: None,
            observation: None,
        });

        for expected in [
            "Profile:              -",
            "Profile revision:     -",
            "Applied revision:     -",
            "Observed at:          -",
            "BIOS version:         -",
            "| (none)    |         |         |         |",
        ] {
            assert!(table.contains(expected), "missing {expected:?} in\n{table}");
        }
    }

    #[test]
    fn attributes_table_shows_constraints() {
        let table = render_attributes(&AttributesReport::from(forgerpc::MachineBiosAttributes {
            registry: "BiosAttributeRegistry.1.0.0".to_string(),
            attributes: vec![
                forgerpc::MachineBiosAttribute {
                    name: "SystemRecovery_POSTWatchdogTimerValue".to_string(),
                    display_name: "POST Watchdog Timer Value".to_string(),
                    r#type: "Integer".to_string(),
                    read_only: false,
                    allowed_values: vec![],
                    lower_bound: Some(5.0),
                    upper_bound: Some(20.0),
                    help_text: None,
                    default_value: Some("5".to_string()),
                },
                forgerpc::MachineBiosAttribute {
                    name: "TrustedComputingGroup_DeviceStatus".to_string(),
                    display_name: "Device Status".to_string(),
                    r#type: "String".to_string(),
                    read_only: true,
                    allowed_values: vec![],
                    lower_bound: None,
                    upper_bound: None,
                    help_text: None,
                    default_value: None,
                },
                forgerpc::MachineBiosAttribute {
                    name: "SystemRecovery_POSTWatchdogTimer".to_string(),
                    display_name: "POST Watchdog Timer".to_string(),
                    r#type: "Enumeration".to_string(),
                    read_only: false,
                    allowed_values: vec!["Disabled".to_string(), "Enabled".to_string()],
                    lower_bound: None,
                    upper_bound: None,
                    help_text: None,
                    default_value: Some("Disabled".to_string()),
                },
            ],
        }));

        for expected in [
            "BIOS attribute registry BiosAttributeRegistry.1.0.0",
            "| Attribute ",
            "| Allowed Values ",
            "| Range ",
            "| Default ",
            "| SystemRecovery_POSTWatchdogTimerValue | POST Watchdog Timer Value | Integer     | false     | -                 | 5..20 | 5        |",
            "| TrustedComputingGroup_DeviceStatus    | Device Status             | String      | true      | -                 | -     | -        |",
            "| SystemRecovery_POSTWatchdogTimer      | POST Watchdog Timer       | Enumeration | false     | Disabled, Enabled | -     | Disabled |",
        ] {
            assert!(table.contains(expected), "missing {expected:?} in\n{table}");
        }
    }
}
