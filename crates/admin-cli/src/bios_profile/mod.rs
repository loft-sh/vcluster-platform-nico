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

mod args;
mod cmd;

use clap::Parser;

use crate::cfg::dispatch::Dispatch;
use crate::cfg::run::Run;
use crate::cfg::runtime::RuntimeContext;
use crate::errors::CarbideCliResult;

#[derive(Parser, Debug, Dispatch)]
#[clap(rename_all = "kebab_case")]
pub(crate) enum Cmd {
    #[clap(about = "List the BIOS profiles stored in NICo")]
    List(args::ListArgs),
    #[clap(about = "Show a stored BIOS profile and its attributes")]
    Show(args::ShowArgs),
    #[clap(about = "Create or replace a stored BIOS profile")]
    Set(args::SetArgs),
    #[clap(about = "Delete a stored BIOS profile")]
    Delete(args::DeleteArgs),
}

impl Run for args::ListArgs {
    async fn run(self, ctx: &mut RuntimeContext) -> CarbideCliResult<()> {
        cmd::handle_list(ctx.config.format, &ctx.api_client).await
    }
}

impl Run for args::ShowArgs {
    async fn run(self, ctx: &mut RuntimeContext) -> CarbideCliResult<()> {
        cmd::handle_show(self, ctx.config.format, &ctx.api_client).await
    }
}

impl Run for args::SetArgs {
    async fn run(self, ctx: &mut RuntimeContext) -> CarbideCliResult<()> {
        cmd::handle_set(self, ctx.config.format, &ctx.api_client).await
    }
}

impl Run for args::DeleteArgs {
    async fn run(self, ctx: &mut RuntimeContext) -> CarbideCliResult<()> {
        cmd::handle_delete(self, &ctx.api_client).await
    }
}
