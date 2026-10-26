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

use carbide_uuid::machine::MachineId;
use clap::{Parser, Subcommand};

#[derive(Subcommand, Debug)]
#[command(after_long_help = "\
EXAMPLES:

Show the BIOS settings NICo last read from a machine and the bios-profile it applies:
    $ nico-admin-cli machine bios show fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg

Read the BIOS settings from the BMC now:
    $ nico-admin-cli machine bios refresh fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg

List the BIOS attributes the BMC offers, as JSON:
    $ nico-admin-cli --format json machine bios attributes \
    fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg

Apply the machine's bios-profile now, while it is in maintenance:
    $ nico-admin-cli machine bios apply fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg

")]
pub(crate) enum Args {
    #[clap(about = "Show the last BIOS settings NICo read and the bios-profile it applies")]
    #[command(after_long_help = "\
EXAMPLES:

Show the stored BIOS observation and profile:
    $ nico-admin-cli machine bios show fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg

As JSON (the global --format flag):
    $ nico-admin-cli --format json machine bios show \
    fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg

")]
    Show(BiosArgs),
    #[clap(
        about = "Read the BIOS settings from the BMC and store them",
        long_about = "Read the BIOS settings from the BMC and store them. A refresh stored less than 60 seconds ago is returned without contacting the BMC."
    )]
    #[command(after_long_help = "\
EXAMPLES:

Read the BIOS settings from the BMC now:
    $ nico-admin-cli machine bios refresh fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg

")]
    Refresh(BiosArgs),
    #[clap(about = "List the BIOS attributes in the BMC's attribute registry")]
    #[command(after_long_help = "\
EXAMPLES:

List every BIOS attribute with its type, allowed values and default:
    $ nico-admin-cli machine bios attributes fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg

As JSON (the global --format flag):
    $ nico-admin-cli --format json machine bios attributes \
    fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg

")]
    Attributes(BiosArgs),
    #[clap(
        about = "Apply the machine's bios-profile now",
        long_about = "Apply the machine's bios-profile now. The machine must be in maintenance, Ready and without an instance; NICo then re-checks its BIOS and boot configuration and re-applies the profile when it differs."
    )]
    #[command(after_long_help = "\
EXAMPLES:

Apply the bios-profile of a machine in maintenance:
    $ nico-admin-cli machine bios apply fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg

")]
    Apply(BiosArgs),
}

#[derive(Parser, Debug)]
pub(crate) struct BiosArgs {
    #[clap(help = "The machine ID")]
    pub(super) machine_id: MachineId,
}
