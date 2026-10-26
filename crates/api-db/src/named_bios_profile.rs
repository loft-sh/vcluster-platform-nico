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

use chrono::{DateTime, Utc};
use model::machine_bios::BiosProfile;
use serde_json::{Map, Value};
use sqlx::PgConnection;
use sqlx::types::Json;

use crate::db_read::DbReader;
use crate::{DatabaseError, DatabaseResult};

#[derive(sqlx::FromRow)]
struct BiosProfileRow {
    name: String,
    vendor: String,
    model: String,
    attribute_registry: Option<String>,
    attributes: Json<Map<String, Value>>,
    revision: String,
    updated_at: DateTime<Utc>,
}

impl From<BiosProfileRow> for BiosProfile {
    fn from(row: BiosProfileRow) -> Self {
        BiosProfile {
            name: row.name,
            vendor: row.vendor,
            model: row.model,
            attribute_registry: row.attribute_registry,
            attributes: row.attributes.0,
            revision: row.revision,
            updated_at: Some(row.updated_at),
        }
    }
}

pub async fn upsert(txn: &mut PgConnection, profile: &BiosProfile) -> DatabaseResult<BiosProfile> {
    let query = "INSERT INTO named_bios_profiles \
         (name, vendor, model, attribute_registry, attributes, revision, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, now()) \
         ON CONFLICT (name) DO UPDATE SET \
         vendor = EXCLUDED.vendor, \
         model = EXCLUDED.model, \
         attribute_registry = EXCLUDED.attribute_registry, \
         attributes = EXCLUDED.attributes, \
         revision = EXCLUDED.revision, \
         updated_at = EXCLUDED.updated_at \
         RETURNING name, vendor, model, attribute_registry, attributes, revision, updated_at";
    sqlx::query_as::<_, BiosProfileRow>(query)
        .bind(&profile.name)
        .bind(&profile.vendor)
        .bind(&profile.model)
        .bind(&profile.attribute_registry)
        .bind(Json(&profile.attributes))
        .bind(&profile.revision)
        .fetch_one(txn)
        .await
        .map(BiosProfile::from)
        .map_err(|e| DatabaseError::query(query, e))
}

pub async fn delete(txn: &mut PgConnection, name: &str) -> DatabaseResult<bool> {
    let query = "DELETE FROM named_bios_profiles WHERE name = $1";
    let result = sqlx::query(query)
        .bind(name)
        .execute(txn)
        .await
        .map_err(|e| DatabaseError::query(query, e))?;
    Ok(result.rows_affected() > 0)
}

pub async fn list(db: impl DbReader<'_>) -> DatabaseResult<Vec<BiosProfile>> {
    let query = "SELECT name, vendor, model, attribute_registry, attributes, revision, updated_at \
         FROM named_bios_profiles ORDER BY name";
    sqlx::query_as::<_, BiosProfileRow>(query)
        .fetch_all(db)
        .await
        .map(|rows| rows.into_iter().map(BiosProfile::from).collect())
        .map_err(|e| DatabaseError::query(query, e))
}

pub async fn get(db: impl DbReader<'_>, name: &str) -> DatabaseResult<Option<BiosProfile>> {
    let query = "SELECT name, vendor, model, attribute_registry, attributes, revision, updated_at \
         FROM named_bios_profiles WHERE name = $1";
    sqlx::query_as::<_, BiosProfileRow>(query)
        .bind(name)
        .fetch_optional(db)
        .await
        .map(|row| row.map(BiosProfile::from))
        .map_err(|e| DatabaseError::query(query, e))
}
