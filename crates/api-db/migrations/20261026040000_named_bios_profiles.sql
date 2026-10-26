CREATE TABLE named_bios_profiles (
    name TEXT PRIMARY KEY,
    vendor TEXT NOT NULL,
    model TEXT NOT NULL,
    attribute_registry TEXT,
    attributes JSONB NOT NULL,
    revision TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE machine_bios_observations ADD COLUMN applied_revision TEXT;
