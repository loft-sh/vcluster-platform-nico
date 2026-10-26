CREATE TABLE machine_bios_observations (
    machine_id VARCHAR(64) PRIMARY KEY
        REFERENCES machines(id) ON UPDATE CASCADE ON DELETE CASCADE,
    observed_at TIMESTAMPTZ NOT NULL,
    observed_by TEXT NOT NULL,
    bios_version TEXT,
    attribute_registry TEXT,
    settings_applied_at TIMESTAMPTZ,
    current JSONB NOT NULL,
    pending JSONB NOT NULL DEFAULT '{}'
);
