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

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(after_long_help = "\
EXAMPLES:

List the BIOS profiles stored in NICo:
    $ nico-admin-cli bios-profile list

")]
pub(crate) struct ListArgs {}

#[derive(Parser, Debug)]
#[command(after_long_help = "\
EXAMPLES:

Show one stored BIOS profile and its attributes:
    $ nico-admin-cli bios-profile show lenovo-h200

")]
pub(crate) struct ShowArgs {
    #[clap(help = "The BIOS profile name, as set in the bios-profile machine label")]
    pub(super) name: String,
}

#[derive(Parser, Debug)]
#[command(after_long_help = "\
EXAMPLES:

Store a BIOS profile from a JSON object of attribute names to values:
    $ nico-admin-cli bios-profile set lenovo-h200 --vendor Lenovo \
    --model 'ThinkSystem SR680a V3' --attributes-file lenovo-h200.json \
    --registry BiosAttributeRegistry.1.0.0

")]
pub(crate) struct SetArgs {
    #[clap(help = "The BIOS profile name, as set in the bios-profile machine label")]
    pub(super) name: String,
    #[clap(long, help = "The host vendor, e.g. Lenovo or Dell")]
    pub(super) vendor: String,
    #[clap(long, help = "The host model, e.g. 'ThinkSystem SR680a V3'")]
    pub(super) model: String,
    #[clap(
        long,
        help = "A JSON file holding an object of BIOS attribute names to string, number or boolean values"
    )]
    pub(super) attributes_file: PathBuf,
    #[clap(long, help = "The BIOS attribute registry the attributes come from")]
    pub(super) registry: Option<String>,
    #[clap(
        long,
        help = "The profile revision; NICo derives one from the profile content when omitted"
    )]
    pub(super) revision: Option<String>,
}

#[derive(Parser, Debug)]
#[command(after_long_help = "\
EXAMPLES:

Delete a stored BIOS profile:
    $ nico-admin-cli bios-profile delete lenovo-h200

")]
pub(crate) struct DeleteArgs {
    #[clap(help = "The BIOS profile name")]
    pub(super) name: String,
}
