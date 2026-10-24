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
use ::rpc::machine_discovery::{Gpu as RpcGpu, GpuPlatformInfo as RpcGpuPlatformInfo};
use carbide_utils::cmd::Cmd;
use serde::Deserialize;

use super::HardwareEnumerationResult;

#[derive(Deserialize)]
struct NvidiaSmiGpu {
    name: String,
    serial: String,
    driver_version: String,
    vbios_version: String,
    inforom_version: String,
    total_memory: String,
    applications_clock: String,
    max_clock: String,
    pci_bus_id: String,
    platform_info: Option<RpcGpuPlatformInfo>,
}

impl From<NvidiaSmiGpu> for RpcGpu {
    fn from(gpu: NvidiaSmiGpu) -> Self {
        let frequency = if gpu.applications_clock.is_empty() {
            gpu.max_clock
        } else {
            gpu.applications_clock
        };
        Self {
            name: gpu.name,
            serial: gpu.serial,
            driver_version: gpu.driver_version,
            vbios_version: gpu.vbios_version,
            inforom_version: gpu.inforom_version,
            total_memory: gpu.total_memory,
            frequency,
            pci_bus_id: gpu.pci_bus_id,
            platform_info: gpu.platform_info,
        }
    }
}

/// Retrieve nvidia-smi data about a machine.
///
/// It is assumed that the machine should have the nvidia kernel module loaded, or this call will fail.
pub(super) fn get_nvidia_smi_data() -> HardwareEnumerationResult<Vec<RpcGpu>> {
    let cmd = Cmd::new("timeout")
        .args(vec![
            "--kill-after=120s",
            "60s",
            "nvidia-smi",
            "--format=csv,noheader",
            concat!(
                "--query-gpu=name,serial,driver_version,vbios_version,inforom.image,memory.total,",
                "clocks.applications.gr,clocks.max.gr,pci.bus_id,platform.chassis_serial_number,",
                "platform.slot_number,platform.tray_index,platform.host_id,platform.module_id,",
                "platform.gpu_fabric_guid"
            ),
        ])
        .output()?;

    Ok(parse_nvidia_smi_csv(&cmd))
}

fn parse_nvidia_smi_csv(output: &str) -> Vec<RpcGpu> {
    let mut csv_reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .trim(csv::Trim::All)
        .from_reader(output.as_bytes());
    let mut gpus = Vec::default();
    for result in csv_reader.records() {
        let gpu = result.and_then(|record| {
            record
                .iter()
                .map(|field| {
                    if field.starts_with('[') && field.ends_with(']') {
                        ""
                    } else {
                        field
                    }
                })
                .collect::<csv::StringRecord>()
                .deserialize::<NvidiaSmiGpu>(None)
        });
        match gpu {
            Ok(gpu) => gpus.push(gpu.into()),
            Err(error) => tracing::error!(%error, "Could not parse nvidia-smi output"),
        }
    }

    gpus
}

#[cfg(test)]
mod tests {
    use carbide_test_support::{Check, check_values};

    use super::*;

    fn h200(frequency: &str) -> RpcGpu {
        RpcGpu {
            name: "NVIDIA H200".to_string(),
            serial: "1754422005699".to_string(),
            driver_version: "615.71.10".to_string(),
            vbios_version: "96.00.CF.00.02".to_string(),
            inforom_version: "G520.0280.02.02".to_string(),
            total_memory: "143771 MiB".to_string(),
            frequency: frequency.to_string(),
            pci_bus_id: "00000000:18:00.0".to_string(),
            platform_info: None,
        }
    }

    #[test]
    fn parse_nvidia_smi_csv_output() {
        check_values(
            [
                Check {
                    scenario: "applications clock reported",
                    input: concat!(
                        "NVIDIA GB200, 1654422005699, 530.30.02, 96.00.30.00.01, 1010.0200.00.02,",
                        "81559 MiB, 1755 MHz, 1980 MHz, 00000000:17:00.0, SN_WHATISTHIS1,",
                        "2, 3, 1, 4, 0xdb488cb17978480"
                    ),
                    expect: vec![RpcGpu {
                        name: "NVIDIA GB200".to_string(),
                        serial: "1654422005699".to_string(),
                        driver_version: "530.30.02".to_string(),
                        vbios_version: "96.00.30.00.01".to_string(),
                        inforom_version: "1010.0200.00.02".to_string(),
                        total_memory: "81559 MiB".to_string(),
                        frequency: "1755 MHz".to_string(),
                        pci_bus_id: "00000000:17:00.0".to_string(),
                        platform_info: Some(RpcGpuPlatformInfo {
                            chassis_serial: "SN_WHATISTHIS1".to_string(),
                            slot_number: 2,
                            tray_index: 3,
                            host_id: 1,
                            module_id: 4,
                            fabric_guid: "0xdb488cb17978480".to_string(),
                        }),
                    }],
                },
                Check {
                    scenario: "applications clock deprecated",
                    input: concat!(
                        "NVIDIA H200, 1754422005699, 615.71.10, 96.00.CF.00.02, G520.0280.02.02,",
                        "143771 MiB, [Requested functionality has been deprecated], 1980 MHz,",
                        "00000000:18:00.0, [N/A], [N/A], [N/A], [N/A], [N/A], [N/A]"
                    ),
                    expect: vec![h200("1980 MHz")],
                },
                Check {
                    scenario: "no clock reported",
                    input: concat!(
                        "NVIDIA H200, 1754422005699, 615.71.10, 96.00.CF.00.02, G520.0280.02.02,",
                        "143771 MiB, [N/A], [Not Supported], 00000000:18:00.0,",
                        "[N/A], [N/A], [N/A], [N/A], [N/A], [N/A]"
                    ),
                    expect: vec![h200("")],
                },
            ],
            parse_nvidia_smi_csv,
        );
    }
}
