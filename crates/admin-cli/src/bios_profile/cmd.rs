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

use std::fmt::Write as _;

use ::rpc::admin_cli::OutputFormat;
use ::rpc::forge as forgerpc;
use prettytable::{Cell, Row, Table};
use serde::Serialize;
use serde_json::{Map, Value};

use super::args::{DeleteArgs, SetArgs, ShowArgs};
use crate::errors::{CarbideCliError, CarbideCliResult};
use crate::rpc::ApiClient;

const SUMMARY_LABEL_WIDTH: usize = 19;

#[derive(Debug, Serialize)]
struct ProfileReport {
    name: String,
    vendor: String,
    model: String,
    attribute_registry: Option<String>,
    revision: String,
    updated_at: Option<String>,
    attributes: Map<String, Value>,
}

impl TryFrom<forgerpc::BiosProfile> for ProfileReport {
    type Error = serde_json::Error;

    fn try_from(profile: forgerpc::BiosProfile) -> Result<Self, Self::Error> {
        Ok(Self {
            name: profile.name,
            vendor: profile.vendor,
            model: profile.model,
            attribute_registry: profile.attribute_registry,
            revision: profile.revision,
            updated_at: profile.updated_at.map(|time| time.to_string()),
            attributes: serde_json::from_str(&profile.attributes_json)?,
        })
    }
}

pub(super) async fn handle_list(
    output_format: OutputFormat,
    api_client: &ApiClient,
) -> CarbideCliResult<()> {
    let profiles = api_client
        .list_bios_profiles()
        .await?
        .profiles
        .into_iter()
        .map(ProfileReport::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    match output_format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&profiles)?),
        OutputFormat::Yaml => println!("{}", serde_yaml::to_string(&profiles)?),
        OutputFormat::AsciiTable | OutputFormat::Csv => print!("{}", render_list(&profiles)),
    }
    Ok(())
}

pub(super) async fn handle_show(
    args: ShowArgs,
    output_format: OutputFormat,
    api_client: &ApiClient,
) -> CarbideCliResult<()> {
    let profile = api_client
        .list_bios_profiles()
        .await?
        .profiles
        .into_iter()
        .find(|profile| profile.name == args.name)
        .ok_or_else(|| {
            CarbideCliError::GenericError(format!("BIOS profile {} not found", args.name))
        })?;
    print_profile(ProfileReport::try_from(profile)?, output_format)
}

pub(super) async fn handle_set(
    args: SetArgs,
    output_format: OutputFormat,
    api_client: &ApiClient,
) -> CarbideCliResult<()> {
    let attributes: Value = serde_json::from_str(&std::fs::read_to_string(&args.attributes_file)?)?;
    let profile = api_client
        .upsert_bios_profile(forgerpc::BiosProfile {
            name: args.name,
            vendor: args.vendor,
            model: args.model,
            attribute_registry: args.registry,
            attributes_json: attributes.to_string(),
            revision: args.revision.unwrap_or_default(),
            updated_at: None,
        })
        .await?;
    print_profile(ProfileReport::try_from(profile)?, output_format)
}

pub(super) async fn handle_delete(
    args: DeleteArgs,
    api_client: &ApiClient,
) -> CarbideCliResult<()> {
    api_client.delete_bios_profile(args.name.clone()).await?;
    println!("Deleted BIOS profile {}", args.name);
    Ok(())
}

fn print_profile(report: ProfileReport, output_format: OutputFormat) -> CarbideCliResult<()> {
    match output_format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        OutputFormat::Yaml => println!("{}", serde_yaml::to_string(&report)?),
        OutputFormat::AsciiTable | OutputFormat::Csv => print!("{}", render_profile(&report)),
    }
    Ok(())
}

fn display_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn render_list(profiles: &[ProfileReport]) -> String {
    let mut table = Table::new();
    table.set_titles(Row::new(
        [
            "Name",
            "Vendor",
            "Model",
            "Registry",
            "Revision",
            "Attributes",
            "Updated",
        ]
        .into_iter()
        .map(Cell::new)
        .collect(),
    ));
    if profiles.is_empty() {
        table.add_row(Row::new(vec![Cell::new("(none)")]));
    }
    for profile in profiles {
        table.add_row(Row::new(vec![
            Cell::new(&profile.name),
            Cell::new(&profile.vendor),
            Cell::new(&profile.model),
            Cell::new(profile.attribute_registry.as_deref().unwrap_or("-")),
            Cell::new(&profile.revision),
            Cell::new(&profile.attributes.len().to_string()),
            Cell::new(profile.updated_at.as_deref().unwrap_or("-")),
        ]));
    }
    table.to_string()
}

fn render_profile(report: &ProfileReport) -> String {
    let mut out = String::new();
    let width = SUMMARY_LABEL_WIDTH;
    let _ = writeln!(out, "BIOS profile {}", report.name);
    for (label, value) in [
        ("Vendor:", report.vendor.as_str()),
        ("Model:", report.model.as_str()),
        (
            "Attribute registry:",
            report.attribute_registry.as_deref().unwrap_or("-"),
        ),
        ("Revision:", report.revision.as_str()),
        ("Updated at:", report.updated_at.as_deref().unwrap_or("-")),
    ] {
        let _ = writeln!(out, "{label:<width$} {value}");
    }
    let mut table = Table::new();
    table.set_titles(Row::new(
        ["Attribute", "Value"].into_iter().map(Cell::new).collect(),
    ));
    if report.attributes.is_empty() {
        table.add_row(Row::new(vec![Cell::new("(none)")]));
    }
    for (name, value) in &report.attributes {
        table.add_row(Row::new(vec![
            Cell::new(name),
            Cell::new(&display_value(value)),
        ]));
    }
    let _ = write!(out, "\n{table}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lenovo_h200() -> ProfileReport {
        ProfileReport::try_from(forgerpc::BiosProfile {
            name: "lenovo-h200".to_string(),
            vendor: "Lenovo".to_string(),
            model: "ThinkSystem SR680a V3".to_string(),
            attribute_registry: Some("BiosAttributeRegistry.1.0.0".to_string()),
            attributes_json: r#"{"OperatingModes_ChooseOperatingMode":"MaximumPerformance","SystemRecovery_POSTWatchdogTimerValue":5}"#.to_string(),
            revision: "3f2a9c1d7b4e".to_string(),
            updated_at: None,
        })
        .unwrap()
    }

    #[test]
    fn profile_shows_its_identity_revision_and_attributes() {
        let rendered = render_profile(&lenovo_h200());

        for expected in [
            "BIOS profile lenovo-h200",
            "Vendor:             Lenovo",
            "Model:              ThinkSystem SR680a V3",
            "Attribute registry: BiosAttributeRegistry.1.0.0",
            "Revision:           3f2a9c1d7b4e",
            "Updated at:         -",
            "| OperatingModes_ChooseOperatingMode    | MaximumPerformance |",
            "| SystemRecovery_POSTWatchdogTimerValue | 5                  |",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected:?} in\n{rendered}"
            );
        }
    }

    #[test]
    fn profile_list_summarizes_each_profile() {
        let rendered = render_list(&[lenovo_h200()]);

        assert!(
            rendered.contains(
                "| lenovo-h200 | Lenovo | ThinkSystem SR680a V3 | BiosAttributeRegistry.1.0.0 | 3f2a9c1d7b4e | 2          | -       |"
            ),
            "{rendered}"
        );
        assert!(render_list(&[]).contains("(none)"));
    }
}
