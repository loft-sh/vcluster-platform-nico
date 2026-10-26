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

use std::str::FromStr;

use carbide_uuid::machine::MachineId;
use chrono::{DateTime, Utc};
use model::machine_bios::{BiosObservationSource, MachineBiosObservation};
use serde_json::{Map, Value};
use sqlx::PgConnection;
use sqlx::types::Json;

use crate::db_read::DbReader;
use crate::{DatabaseError, DatabaseResult};

#[derive(sqlx::FromRow)]
struct ObservationRow {
    machine_id: MachineId,
    observed_at: DateTime<Utc>,
    observed_by: String,
    bios_version: Option<String>,
    attribute_registry: Option<String>,
    settings_applied_at: Option<DateTime<Utc>>,
    current: Json<Map<String, Value>>,
    pending: Json<Map<String, Value>>,
}

impl TryFrom<ObservationRow> for MachineBiosObservation {
    type Error = DatabaseError;

    fn try_from(row: ObservationRow) -> Result<Self, Self::Error> {
        let observed_by = BiosObservationSource::from_str(&row.observed_by).map_err(|_| {
            DatabaseError::Internal {
                message: format!(
                    "machine {} has a BIOS observation from unknown source {}",
                    row.machine_id, row.observed_by
                ),
            }
        })?;
        Ok(MachineBiosObservation {
            machine_id: row.machine_id,
            observed_at: row.observed_at,
            observed_by,
            bios_version: row.bios_version,
            attribute_registry: row.attribute_registry,
            settings_applied_at: row.settings_applied_at,
            current: row.current.0,
            pending: row.pending.0,
        })
    }
}

pub async fn upsert(
    txn: &mut PgConnection,
    observation: &MachineBiosObservation,
) -> DatabaseResult<()> {
    let query = "INSERT INTO machine_bios_observations \
         (machine_id, observed_at, observed_by, bios_version, attribute_registry, \
         settings_applied_at, current, pending) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (machine_id) DO UPDATE SET \
         observed_at = EXCLUDED.observed_at, \
         observed_by = EXCLUDED.observed_by, \
         bios_version = EXCLUDED.bios_version, \
         attribute_registry = EXCLUDED.attribute_registry, \
         settings_applied_at = EXCLUDED.settings_applied_at, \
         current = EXCLUDED.current, \
         pending = EXCLUDED.pending";
    sqlx::query(query)
        .bind(observation.machine_id)
        .bind(observation.observed_at)
        .bind(observation.observed_by.as_str())
        .bind(&observation.bios_version)
        .bind(&observation.attribute_registry)
        .bind(observation.settings_applied_at)
        .bind(Json(&observation.current))
        .bind(Json(&observation.pending))
        .execute(txn)
        .await
        .map_err(|e| DatabaseError::query(query, e))?;
    Ok(())
}

pub async fn find_by_machine_id(
    db: impl DbReader<'_>,
    machine_id: &MachineId,
) -> DatabaseResult<Option<MachineBiosObservation>> {
    let query = "SELECT machine_id, observed_at, observed_by, bios_version, attribute_registry, \
         settings_applied_at, current, pending \
         FROM machine_bios_observations WHERE machine_id = $1";
    sqlx::query_as::<_, ObservationRow>(query)
        .bind(machine_id)
        .fetch_optional(db)
        .await
        .map_err(|e| DatabaseError::query(query, e))?
        .map(MachineBiosObservation::try_from)
        .transpose()
}
